//! A login through PAM: the password checked, the account looked at, the
//! session opened, and what PAM's modules want the session to have in its
//! environment handed out (logind's variables, `pam_env`'s, a keyring's
//! socket). The library's own client does the first three and keeps the
//! environment to itself, so the calls are made here.

use pam::{PamFlag, PamHandle, PamMessage, PamMessageStyle, PamResponse, PamReturnCode};
use std::ffi::{c_int, c_void, CStr, CString};

/// The name and the password PAM is answered with.
struct Credentials {
    user: CString,
    password: CString,
}

/// PAM's questions, answered: what is asked for openly is the name, what
/// is asked for blindly is the password; what it only says is logged.
///
/// # Safety
/// Called by PAM with `count` messages in `messages`, room for the
/// address of as many answers in `answers`, and `data` as it was given
/// to `pam_start`: a `Credentials` that lives as long as the handle.
unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const PamMessage,
    answers: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if count <= 0 || messages.is_null() || answers.is_null() || data.is_null() {
        return PamReturnCode::Conv_Err as c_int;
    }
    // (PAM frees the answers, and each answer's text, with `free`.)
    let list = libc::calloc(count as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if list.is_null() {
        return PamReturnCode::Buf_Err as c_int;
    }
    let credentials = &*(data as *const Credentials);
    for index in 0..count as isize {
        let message = &**messages.offset(index);
        let answer = &mut *list.offset(index);
        match PamMessageStyle::from(message.msg_style) {
            // (What PAM is answered is never said; what it asks, only by
            // its kind.)
            PamMessageStyle::Prompt_Echo_On => {
                log::debug!("PAM asks for the name");
                answer.resp = libc::strdup(credentials.user.as_ptr());
            }
            PamMessageStyle::Prompt_Echo_Off => {
                log::debug!("PAM asks for the password");
                answer.resp = libc::strdup(credentials.password.as_ptr());
            }
            PamMessageStyle::Text_Info | PamMessageStyle::Error_Msg => {
                if !message.msg.is_null() {
                    log::info!("PAM: {}", CStr::from_ptr(message.msg).to_string_lossy());
                }
            }
        }
    }
    *answers = list;
    PamReturnCode::Success as c_int
}

/// One login's dealings with PAM. Dropped, the session it opened is
/// closed and PAM is done with.
pub struct Session {
    handle: &'static mut PamHandle,
    // (PAM keeps the addresses of both for as long as the handle lives.)
    _conversation: Box<pam::ffi::pam_conv>,
    _credentials: Box<Credentials>,
    opened: bool,
}

impl Session {
    /// The password is not needed any more (the session is open): what
    /// was kept of it for PAM's questions is overwritten. (It stayed in
    /// the worker's memory for as long as the person was logged in.)
    pub fn forget_password(&mut self) {
        let mut bytes = std::mem::take(&mut self._credentials.password).into_bytes();
        bytes.fill(0);
    }

    /// Begin with a service of `/etc/pam.d` and what was typed.
    pub fn start(service: &str, user: &str, password: &str) -> Result<Self, String> {
        let credentials = Box::new(Credentials {
            user: CString::new(user).map_err(|_| "a name with a nought in it".to_string())?,
            password: CString::new(password)
                .map_err(|_| "a password with a nought in it".to_string())?,
        });
        let conversation = Box::new(pam::ffi::pam_conv {
            conv: Some(converse),
            appdata_ptr: std::ptr::from_ref::<Credentials>(&credentials) as *mut c_void,
        });
        // (No name is given here: PAM asks for it, and is answered. The
        // library's `start` hands PAM the address of a name it has
        // already let go of, and PAM then knew the person by whatever
        // lay there: the service's own name, when it was tried.)
        let handle = pam::start(service, None, &conversation)
            .map_err(|error| format!("PAM did not start: {error}"))?;
        Ok(Self {
            handle,
            _conversation: conversation,
            _credentials: credentials,
            opened: false,
        })
    }

    /// Check the password, and that the account may log in now.
    pub fn authenticate(&mut self) -> Result<(), PamReturnCode> {
        let code = pam::authenticate(self.handle, PamFlag::None);
        log::debug!("PAM's check of the password: {code:?}");
        if code != PamReturnCode::Success {
            return Err(code);
        }
        let code = pam::acct_mgmt(self.handle, PamFlag::None);
        log::debug!("PAM's look at the account: {code:?}");
        if code != PamReturnCode::Success {
            return Err(code);
        }
        Ok(())
    }

    /// Open the session (credentials set before and after, as OpenSSH
    /// does it).
    pub fn open(&mut self) -> Result<(), PamReturnCode> {
        let code = pam::setcred(self.handle, PamFlag::Establish_Cred);
        log::debug!("PAM set the credentials: {code:?}");
        if code != PamReturnCode::Success {
            return Err(code);
        }
        let code = pam::open_session(self.handle, false);
        log::debug!("PAM opened the session: {code:?}");
        if code != PamReturnCode::Success {
            pam::setcred(self.handle, PamFlag::Delete_Cred);
            return Err(code);
        }
        self.opened = true;
        let code = pam::setcred(self.handle, PamFlag::Reinitialize_Cred);
        log::debug!("PAM set the credentials again: {code:?}");
        if code != PamReturnCode::Success {
            return Err(code);
        }
        Ok(())
    }

    /// What PAM's modules want the session's programs to have in their
    /// environment. (Read from PAM's own list here: the library's reader
    /// of it keeps the `=` with each name and drops the first letter of
    /// each value, which made `XDG_SEAT` something no seat is called.)
    pub fn environment(&mut self) -> Vec<(String, String)> {
        // SAFETY: PAM hands out a list of texts of its own making, ended
        // by a null, each to be freed by the caller, and the list too.
        unsafe {
            let list = pam::ffi::pam_getenvlist(self.handle);
            if list.is_null() {
                return Vec::new();
            }
            let mut found = Vec::new();
            let mut at = list;
            while !(*at).is_null() {
                if let Some(pair) = pair_of(CStr::from_ptr(*at).to_bytes()) {
                    found.push(pair);
                }
                libc::free(*at as *mut c_void);
                at = at.add(1);
            }
            libc::free(list as *mut c_void);
            found
        }
    }
}

/// One line of an environment, `NAME=value`, as its two parts. A line
/// without a name, without the sign, or that is not text is none.
pub fn pair_of(line: &[u8]) -> Option<(String, String)> {
    let line = std::str::from_utf8(line).ok()?;
    let (name, value) = line.split_once('=')?;
    (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.opened {
            let code = pam::close_session(self.handle, false);
            log::debug!("PAM closed the session: {code:?}");
        }
        let code = pam::setcred(self.handle, PamFlag::Delete_Cred);
        log::debug!("PAM took back the credentials: {code:?}");
        pam::end(self.handle, code);
    }
}

/// The session's environment: what gevrek says of the session itself,
/// then what PAM's modules add or say otherwise (logind's own word on
/// the session's number and its runtime directory, `pam_env`'s
/// variables, a keyring's socket). Names that could turn the session's
/// first program into another one are not taken from PAM.
pub fn merged(own: &[(String, String)], from_pam: &[(String, String)]) -> Vec<(String, String)> {
    const NOT_FROM_PAM: &[&str] = &["LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "IFS"];
    let mut all: Vec<(String, String)> = own.to_vec();
    for (name, value) in from_pam {
        if name.is_empty() || NOT_FROM_PAM.contains(&name.as_str()) {
            continue;
        }
        match all.iter_mut().find(|(known, _)| known == name) {
            Some((_, known)) => known.clone_from(value),
            None => all.push((name.clone(), value.clone())),
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::{merged, pair_of};

    #[test]
    fn a_line_of_an_environment_is_a_name_and_a_value() {
        let pair = |name: &str, value: &str| Some((name.to_string(), value.to_string()));
        assert_eq!(pair_of(b"XDG_SEAT=seat0"), pair("XDG_SEAT", "seat0"));
        assert_eq!(pair_of(b"EMPTY="), pair("EMPTY", ""));
        assert_eq!(pair_of(b"A=b=c"), pair("A", "b=c"));
        assert_eq!(pair_of(b"=nameless"), None);
        assert_eq!(pair_of(b"no sign"), None);
        assert_eq!(pair_of(b"BAD=\xff\xfe"), None);
    }

    #[test]
    fn what_pam_says_of_the_environment_is_added_and_goes_before_gevreks_own() {
        let pair = |name: &str, value: &str| (name.to_string(), value.to_string());
        let own = vec![
            pair("XDG_SEAT", "seat0"),
            pair("XDG_RUNTIME_DIR", "/run/user/1"),
        ];
        let from_pam = vec![
            pair("XDG_RUNTIME_DIR", "/run/user/1000"),
            pair("XDG_SESSION_ID", "7"),
            pair("GNOME_KEYRING_CONTROL", "/run/user/1000/keyring"),
            pair("LD_PRELOAD", "/tmp/evil.so"),
            pair("", "nothing"),
        ];
        assert_eq!(
            merged(&own, &from_pam),
            vec![
                pair("XDG_SEAT", "seat0"),
                pair("XDG_RUNTIME_DIR", "/run/user/1000"),
                pair("XDG_SESSION_ID", "7"),
                pair("GNOME_KEYRING_CONTROL", "/run/user/1000/keyring"),
            ]
        );
        assert_eq!(merged(&own, &[]), own);
    }
}
