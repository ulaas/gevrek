use anyhow::{anyhow, Result};
use gevrek_model::settings::{LaunchProfile, StartupProfile, StartupRestartPolicy};
use std::collections::{BTreeSet, HashMap};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupProcessState {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub status: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

struct ProcessRecord {
    child: Option<Child>,
    status: String,
    next_launch: Option<Instant>,
    restart_count: u32,
    last_exit_code: Option<i32>,
    last_error: Option<String>,
    suppress_restart: bool,
}

impl ProcessRecord {
    fn scheduled(at: Instant) -> Self {
        Self {
            child: None,
            status: "scheduled".to_string(),
            next_launch: Some(at),
            restart_count: 0,
            last_exit_code: None,
            last_error: None,
            suppress_restart: false,
        }
    }

    fn disabled() -> Self {
        Self {
            child: None,
            status: "disabled".to_string(),
            next_launch: None,
            restart_count: 0,
            last_exit_code: None,
            last_error: None,
            suppress_restart: true,
        }
    }
}

pub struct StartupProcessManager {
    profiles: HashMap<String, StartupProfile>,
    order: Vec<String>,
    records: HashMap<String, ProcessRecord>,
}

impl Default for StartupProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StartupProcessManager {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
            order: Vec::new(),
            records: HashMap::new(),
        }
    }

    pub fn configure(&mut self, profiles: &[StartupProfile], reload: bool) {
        let now = Instant::now();
        let incoming = profiles
            .iter()
            .map(|profile| (normalize_id(&profile.id), profile.clone()))
            .collect::<HashMap<_, _>>();
        let incoming_ids = incoming.keys().cloned().collect::<BTreeSet<_>>();
        let removed = self
            .profiles
            .keys()
            .filter(|id| !incoming_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in removed {
            if self
                .profiles
                .get(&id)
                .is_some_and(|profile| profile.terminate_on_exit)
            {
                if let Err(error) = self.stop_record(&id, "removed") {
                    log::debug!(
                        "startup application '{id}' was removed but not stopped: {error:#}"
                    );
                }
            }
            self.records.remove(&id);
        }

        self.order = profiles
            .iter()
            .map(|profile| normalize_id(&profile.id))
            .collect();
        // The ones that were switched off until now.
        let were_off: BTreeSet<String> = self
            .profiles
            .iter()
            .filter(|(_, profile)| !profile.enabled)
            .map(|(id, _)| id.clone())
            .collect();
        self.profiles = incoming;

        for id in self.order.clone() {
            let Some(profile) = self.profiles.get(&id).cloned() else {
                continue;
            };
            if !profile.enabled {
                if profile.terminate_on_exit {
                    if let Err(error) = self.stop_record(&id, "disabled") {
                        log::debug!(
                            "startup application '{id}' was turned off but not stopped: {error:#}"
                        );
                    }
                }
                self.records.insert(id, ProcessRecord::disabled());
                continue;
            }

            if reload && profile.restart_on_reload {
                if let Err(error) = self.stop_record(&id, "reloading") {
                    log::debug!(
                        "startup application '{id}' is started again but was not stopped: {error:#}"
                    );
                }
                self.records.insert(
                    id,
                    ProcessRecord::scheduled(now + Duration::from_millis(profile.delay_ms.into())),
                );
            } else if were_off.contains(&id) {
                // Switched on now: it is started, as one that is new.
                // (What was kept of it said "disabled" and nothing
                // more, and it was not started until gevrek was.)
                self.records.insert(
                    id,
                    ProcessRecord::scheduled(now + Duration::from_millis(profile.delay_ms.into())),
                );
            } else {
                self.records.entry(id).or_insert_with(|| {
                    ProcessRecord::scheduled(now + Duration::from_millis(profile.delay_ms.into()))
                });
            }
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        for id in self.order.clone() {
            self.observe_exit(&id, now);
            let due = self
                .records
                .get(&id)
                .and_then(|record| record.next_launch)
                .is_some_and(|deadline| deadline <= now);
            if due {
                let first = self
                    .records
                    .get(&id)
                    .is_none_or(|record| record.restart_count == 0);
                if let Err(error) = self.spawn_profile(&id) {
                    // Said once; the tries after it are at debug.
                    if first {
                        log::warn!("{error:#}");
                    } else {
                        log::debug!("{error:#}");
                    }
                }
            }
        }
    }

    pub fn launch(&mut self, id: &str) -> Result<()> {
        let id = self.resolve_id(id)?;
        if self
            .records
            .get(&id)
            .and_then(|record| record.child.as_ref())
            .is_some()
        {
            return Err(anyhow!("Startup application '{id}' is already running"));
        }
        if let Some(record) = self.records.get_mut(&id) {
            record.suppress_restart = false;
            record.restart_count = 0;
            record.next_launch = None;
        }
        self.spawn_profile(&id)
    }

    pub fn stop(&mut self, id: &str) -> Result<()> {
        let id = self.resolve_id(id)?;
        self.stop_record(&id, "stopped")
    }

    pub fn restart(&mut self, id: &str) -> Result<()> {
        let id = self.resolve_id(id)?;
        // One that is switched off is not started, again or at all. (It
        // was set to start at once, could not be, and said so with
        // every pass of the loop.)
        if self
            .profiles
            .get(&id)
            .is_some_and(|profile| !profile.enabled)
        {
            return Err(anyhow!("Startup application '{id}' is disabled"));
        }
        self.stop_record(&id, "restarting")?;
        if let Some(record) = self.records.get_mut(&id) {
            record.suppress_restart = false;
            record.restart_count = 0;
            record.next_launch = Some(Instant::now());
            record.status = "scheduled".to_string();
        }
        Ok(())
    }

    pub fn states(&self) -> Vec<StartupProcessState> {
        self.order
            .iter()
            .filter_map(|id| {
                let profile = self.profiles.get(id)?;
                let record = self.records.get(id);
                Some(StartupProcessState {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    enabled: profile.enabled,
                    status: record
                        .map(|record| record.status.clone())
                        .unwrap_or_else(|| "idle".to_string()),
                    pid: record
                        .and_then(|record| record.child.as_ref())
                        .map(Child::id),
                    restart_count: record.map_or(0, |record| record.restart_count),
                    last_exit_code: record.and_then(|record| record.last_exit_code),
                    last_error: record.and_then(|record| record.last_error.clone()),
                })
            })
            .collect()
    }

    pub fn shutdown(&mut self) {
        let ids = self.order.clone();
        for id in ids {
            if self
                .profiles
                .get(&id)
                .is_some_and(|profile| profile.terminate_on_exit)
            {
                if let Err(error) = self.stop_record(&id, "stopped") {
                    log::debug!("startup application '{id}' was not stopped: {error:#}");
                }
            }
        }
    }

    fn resolve_id(&self, id: &str) -> Result<String> {
        let id = normalize_id(id);
        if id.is_empty() {
            return Err(anyhow!("Startup application ID cannot be empty"));
        }
        self.profiles
            .contains_key(&id)
            .then_some(id.clone())
            .ok_or_else(|| anyhow!("No startup application named '{id}' exists"))
    }

    fn spawn_profile(&mut self, id: &str) -> Result<()> {
        let profile = self
            .profiles
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("No startup application named '{id}' exists"))?;
        if !profile.enabled {
            return Err(anyhow!("Startup application '{}' is disabled", profile.id));
        }

        match spawn_launch(&profile.launch) {
            Ok(child) => {
                let pid = child.id();
                if profile.terminate_on_exit {
                    end_with_this_program(&child);
                }
                let record = self
                    .records
                    .entry(id.to_string())
                    .or_insert_with(ProcessRecord::disabled);
                record.child = Some(child);
                record.status = "running".to_string();
                record.next_launch = None;
                record.last_error = None;
                record.last_exit_code = None;
                record.suppress_restart = false;
                log::info!(
                    "startup application '{}' was started as process {pid} ({})",
                    profile.id,
                    profile.launch.program.trim()
                );
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                let record = self
                    .records
                    .entry(id.to_string())
                    .or_insert_with(ProcessRecord::disabled);
                record.child = None;
                record.status = "failed".to_string();
                record.last_error = Some(message.clone());
                record.next_launch = None;
                if should_restart(&profile, None, record.restart_count) {
                    record.restart_count = record.restart_count.saturating_add(1);
                    record.status = "restart pending".to_string();
                    record.next_launch = Some(
                        Instant::now() + Duration::from_millis(profile.restart_delay_ms.into()),
                    );
                    log::debug!(
                        "startup application '{}' is tried again in {} ms (try {})",
                        profile.id,
                        profile.restart_delay_ms,
                        record.restart_count
                    );
                }
                Err(anyhow!(
                    "Could not launch startup application '{}': {}",
                    profile.id,
                    message
                ))
            }
        }
    }

    fn observe_exit(&mut self, id: &str, now: Instant) {
        let exit = {
            let Some(record) = self.records.get_mut(id) else {
                return;
            };
            let Some(child) = record.child.as_mut() else {
                return;
            };
            match child.try_wait() {
                Ok(Some(status)) => Some(Ok(status)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
        };
        let Some(exit) = exit else {
            return;
        };
        let Some(profile) = self.profiles.get(id).cloned() else {
            return;
        };
        let record = self.records.get_mut(id).expect("record disappeared");
        let pid = record.child.as_ref().map(Child::id);
        record.child = None;
        match exit {
            Ok(status) => {
                log::debug!(
                    "startup application '{}' (process {}) ended: {status}",
                    profile.id,
                    pid.unwrap_or_default()
                );
                record.last_exit_code = status.code();
                record.last_error = None;
                let restart = !record.suppress_restart
                    && should_restart(&profile, Some(status), record.restart_count);
                if restart {
                    record.restart_count = record.restart_count.saturating_add(1);
                    record.status = "restart pending".to_string();
                    record.next_launch =
                        Some(now + Duration::from_millis(profile.restart_delay_ms.into()));
                    log::debug!(
                        "startup application '{}' is started again in {} ms (restart {})",
                        profile.id,
                        profile.restart_delay_ms,
                        record.restart_count
                    );
                } else {
                    record.status = if status.success() {
                        "exited".to_string()
                    } else {
                        "failed".to_string()
                    };
                    record.next_launch = None;
                }
            }
            Err(error) => {
                log::debug!(
                    "whether startup application '{}' still runs is not known: {error}",
                    profile.id
                );
                record.status = "failed".to_string();
                record.last_error = Some(error.to_string());
                record.next_launch = None;
            }
        }
    }

    fn stop_record(&mut self, id: &str, status: &str) -> Result<()> {
        let record = self
            .records
            .entry(id.to_string())
            .or_insert_with(ProcessRecord::disabled);
        record.suppress_restart = true;
        record.next_launch = None;
        if let Some(mut child) = record.child.take() {
            let pid = child.id();
            if let Err(error) = child.kill() {
                // Still running, and still ours to watch: it is kept.
                // (It was let go of, and a second copy was started
                // beside it later.)
                record.last_error = Some(error.to_string());
                record.child = Some(child);
                return Err(anyhow!(
                    "Could not stop startup application '{id}': {error}"
                ));
            }
            match child.wait() {
                Ok(exit) => {
                    log::debug!("startup application '{id}' (process {pid}) was stopped ({status}): {exit}")
                }
                Err(error) => log::debug!(
                    "startup application '{id}' (process {pid}) was stopped ({status}), its end not seen: {error}"
                ),
            }
        }
        record.status = status.to_string();
        Ok(())
    }
}

/// Have a started program end when this one does, however this one ends
/// (stopped, ended by force, fallen over), and what it started with it.
/// Without this a program that was to end with gevrek outlived a gevrek
/// that fell over, and the next gevrek started a second one beside it.
#[cfg(windows)]
fn end_with_this_program(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    // One job for all of them, held for as long as this program lives:
    // Windows closes it when the program is gone, and ends what is in it.
    static JOB: std::sync::OnceLock<Option<isize>> = std::sync::OnceLock::new();
    let job = *JOB.get_or_init(|| {
        // SAFETY: a job of this program's own, never closed by it; the
        // limits are a plain structure that lives until the call returns.
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(job.0 as isize)
        }
    });
    let Some(job) = job else {
        log::debug!("no job for the started programs: they are ended one by one");
        return;
    };
    // SAFETY: both handles are valid for the length of the call.
    let joined = unsafe {
        AssignProcessToJobObject(
            HANDLE(job as *mut _),
            HANDLE(child.as_raw_handle() as *mut _),
        )
    };
    if let Err(error) = joined {
        log::debug!(
            "process {} does not end with gevrek by itself: {error}",
            child.id()
        );
    }
}

#[cfg(not(windows))]
fn end_with_this_program(_child: &Child) {}

fn normalize_id(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

pub fn spawn_launch(launch: &LaunchProfile) -> Result<Child> {
    let program = launch.program.trim();
    if program.is_empty() {
        return Err(anyhow!("Launch program cannot be empty"));
    }
    let mut command = Command::new(program);
    command.args(&launch.args);
    command.envs(&launch.environment);
    if let Some(directory) = launch
        .working_directory
        .as_deref()
        .map(str::trim)
        .filter(|directory| !directory.is_empty())
    {
        command.current_dir(directory);
    }
    #[cfg(windows)]
    if launch.create_no_window {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map_err(Into::into)
}

fn should_restart(
    profile: &StartupProfile,
    status: Option<ExitStatus>,
    restart_count: u32,
) -> bool {
    let within_limit = profile
        .max_restarts
        .is_none_or(|limit| restart_count < limit);
    within_limit
        && match profile.restart {
            StartupRestartPolicy::Never => false,
            StartupRestartPolicy::Always => true,
            StartupRestartPolicy::OnFailure => status.is_none_or(|status| !status.success()),
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_that_is_switched_on_by_a_reload_is_started_and_one_that_is_off_is_not_restarted() {
        let off = StartupProfile {
            id: "notes".to_string(),
            enabled: false,
            ..StartupProfile::default()
        };
        let mut manager = StartupProcessManager::new();
        manager.configure(std::slice::from_ref(&off), false);
        assert_eq!(manager.records["notes"].status, "disabled");
        // Off: asked to start again, it says no and nothing is due.
        assert!(manager.restart("notes").is_err());
        assert!(manager.records["notes"].next_launch.is_none());
        // Switched on by a reload: due to be started.
        let on = StartupProfile {
            enabled: true,
            ..off
        };
        manager.configure(&[on], true);
        assert!(manager.records["notes"].next_launch.is_some());
    }

    #[test]
    fn startup_ids_are_normalized_for_control() {
        let mut manager = StartupProcessManager::new();
        manager.configure(
            &[StartupProfile {
                id: "  Status-Bar ".to_string(),
                enabled: false,
                ..StartupProfile::default()
            }],
            false,
        );
        assert_eq!(manager.resolve_id("STATUS-BAR").unwrap(), "status-bar");
        assert_eq!(manager.states()[0].status, "disabled");
    }

    #[test]
    fn restart_limit_is_respected_before_spawning_again() {
        let profile = StartupProfile {
            restart: StartupRestartPolicy::Always,
            max_restarts: Some(2),
            ..StartupProfile::default()
        };
        assert!(should_restart(&profile, None, 0));
        assert!(should_restart(&profile, None, 1));
        assert!(!should_restart(&profile, None, 2));

        let profile = StartupProfile {
            restart: StartupRestartPolicy::OnFailure,
            ..profile
        };
        assert!(should_restart(&profile, None, 0));
    }

    #[cfg(windows)]
    #[test]
    fn structured_launch_reports_real_exit_state() {
        let mut manager = StartupProcessManager::new();
        manager.configure(
            &[StartupProfile {
                id: "exit-probe".to_string(),
                name: "Exit probe".to_string(),
                launch: LaunchProfile {
                    program: "cmd.exe".to_string(),
                    args: vec!["/C".to_string(), "exit".to_string(), "7".to_string()],
                    create_no_window: true,
                    ..LaunchProfile::default()
                },
                ..StartupProfile::default()
            }],
            false,
        );
        manager.tick();
        assert!(manager.states()[0].pid.is_some());

        for _ in 0..100 {
            manager.tick();
            if manager.states()[0].last_exit_code.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = &manager.states()[0];
        assert_eq!(state.status, "failed");
        assert_eq!(state.last_exit_code, Some(7));
        assert!(state.pid.is_none());
    }
}
