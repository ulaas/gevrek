//! Applications that are started by a message on the session's bus.
//!
//! A desktop entry with `DBusActivatable=true` says its application is on
//! the bus under the entry's name and answers `org.freedesktop.Application`
//! there; the bus starts it if it is not running. Started so, an
//! application that is open already is told to show itself instead of a
//! second copy being run, and it runs in the session's environment and not
//! in whatever the one who asked had.

use crate::launcher::Activation;
use std::collections::HashMap;
use zbus::zvariant::Value;

const APPLICATION: &str = "org.freedesktop.Application";

/// The method and whether it names an action, for what is asked.
fn method_of(activation: &Activation) -> &'static str {
    match activation.action {
        Some(_) => "ActivateAction",
        None => "Activate",
    }
}

fn ask(activation: &Activation) -> anyhow::Result<()> {
    let path = gevrek_system::apps::bus_path_of(&activation.app)
        .ok_or_else(|| anyhow::anyhow!("{} is no name on the bus", activation.app))?;
    // The bus answers by itself where there is no such service or it does
    // not come up (it gives one 25 seconds), so the wait has an end.
    let connection = zbus::blocking::Connection::session()?;
    // What the application is told of where it is started from: nothing.
    let platform: HashMap<&str, Value<'_>> = HashMap::new();
    let method = method_of(activation);
    match &activation.action {
        Some(action) => {
            let parameters: Vec<Value<'_>> = Vec::new();
            connection.call_method(
                Some(activation.app.as_str()),
                path.as_str(),
                Some(APPLICATION),
                method,
                &(action.as_str(), parameters, platform),
            )?;
        }
        None => {
            connection.call_method(
                Some(activation.app.as_str()),
                path.as_str(),
                Some(APPLICATION),
                method,
                &(platform,),
            )?;
        }
    }
    Ok(())
}

/// Start the application over the bus, without holding the caller up.
/// `otherwise` is what to do where the bus does not start it (no such
/// service installed, or it does not answer): the entry's own command.
pub fn start(activation: Activation, otherwise: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("gevrek-activate".to_string())
        .spawn(move || {
            if let Err(error) = ask(&activation) {
                log::warn!(
                    "{} was not started over the bus ({error}); its command is run instead",
                    activation.app
                );
                otherwise();
            }
        });
    if let Err(error) = spawned {
        log::warn!("no thread to start an application over the bus: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_is_asked_for_by_its_own_method() {
        let app = Activation {
            app: "org.gnome.Calculator".to_string(),
            action: None,
        };
        assert_eq!(method_of(&app), "Activate");
        let action = Activation {
            action: Some("new".to_string()),
            ..app
        };
        assert_eq!(method_of(&action), "ActivateAction");
    }

    struct Heard(std::sync::mpsc::Sender<String>);

    #[zbus::interface(name = "org.freedesktop.Application")]
    impl Heard {
        fn activate(&self, _platform: HashMap<String, zbus::zvariant::OwnedValue>) {
            let _ = self.0.send("activate".to_string());
        }

        fn activate_action(
            &self,
            action: String,
            _parameters: Vec<zbus::zvariant::OwnedValue>,
            _platform: HashMap<String, zbus::zvariant::OwnedValue>,
        ) {
            let _ = self.0.send(format!("action {action}"));
        }
    }

    /// Against a bus of its own, never the session's:
    /// `dbus-run-session -- cargo test -p gevrek-shell over_a_bus -- --ignored`
    #[test]
    #[ignore = "needs a session bus of its own (dbus-run-session)"]
    fn an_application_hears_what_is_asked_over_a_bus() {
        let (tell, heard) = std::sync::mpsc::channel();
        let _service = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name("org.gevrek.Activate-Check"))
            .and_then(|builder| builder.serve_at("/org/gevrek/Activate_Check", Heard(tell)))
            .and_then(|builder| builder.build())
            .expect("a name on the bus");
        let app = Activation {
            app: "org.gevrek.Activate-Check".to_string(),
            action: None,
        };
        ask(&app).expect("the application is asked");
        ask(&Activation {
            action: Some("new".to_string()),
            ..app
        })
        .expect("its action is asked for");
        let wait = std::time::Duration::from_secs(5);
        assert_eq!(heard.recv_timeout(wait).as_deref(), Ok("activate"));
        assert_eq!(heard.recv_timeout(wait).as_deref(), Ok("action new"));
        // One that is not there: an error (and so the command is run),
        // not a wait without end.
        assert!(ask(&Activation {
            app: "org.gevrek.No-Such".to_string(),
            action: None,
        })
        .is_err());
    }
}
