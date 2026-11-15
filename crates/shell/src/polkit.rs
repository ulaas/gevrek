//! The authentication agent: when a program asks the system for something
//! that needs the user's word (installing software, a disk that is not
//! theirs), polkit asks the session's agent to get it. This is that agent.
//!
//! It registers with polkit on the system's bus for this login session.
//! For a request it shows who asks for what, and lets polkit's own helper
//! (the only thing that can check the user and tell polkit) lead from
//! there: what the system's check asks for is asked of the user, one
//! question after another (a password, a code), and what it only says
//! is shown (a finger to be put on a reader). The agent never learns
//! whether an answer was right other than by the helper's verdict, and
//! keeps an answer no longer than it takes to hand it on.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const AGENT_PATH: &str = "/org/gevrek/PolicyKit1/AuthenticationAgent";

/// Where polkit's helper is on the systems gevrek knows.
const HELPERS: [&str; 4] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
    "/usr/lib/x86_64-linux-gnu/polkit-1/polkit-agent-helper-1",
];

/// A request for the user's word.
pub struct Ask {
    /// What is asked for, in polkit's words ("Authentication is needed to ...").
    pub message: String,
    /// The action's name (`org.freedesktop.policykit.exec`).
    pub action: String,
    /// The account whose password is wanted.
    pub user: String,
    /// What polkit knows the request by.
    pub cookie: String,
    /// Tell polkit's caller how it ended: `true` when the helper said yes.
    pub answer: async_channel::Sender<bool>,
}

/// What the agent on the bus tells the shell.
pub enum FromAgent {
    Begin(Ask),
    /// polkit took a request back.
    Cancel(String),
}

struct Agent<F: Fn(FromAgent) + Send + Sync + 'static> {
    tell: F,
}

type Identity = (
    String,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
);

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl<F: Fn(FromAgent) + Send + Sync + 'static> Agent<F> {
    async fn begin_authentication(
        &self,
        action_id: &str,
        message: &str,
        _icon_name: &str,
        _details: std::collections::HashMap<String, String>,
        cookie: &str,
        identities: Vec<Identity>,
    ) -> zbus::fdo::Result<()> {
        // The accounts that may answer: this user when it is among them
        // (an administrator), else the first (usually root).
        let uids: Vec<u32> = identities
            .iter()
            .filter(|(kind, _)| kind == "unix-user")
            .filter_map(|(_, details)| {
                details
                    .get("uid")
                    .and_then(|value| u32::try_from(value).ok())
            })
            .collect();
        let own = own_uid();
        let uid = uids
            .iter()
            .copied()
            .find(|uid| Some(*uid) == own)
            .or_else(|| uids.first().copied());
        let Some(user) = uid.and_then(user_name) else {
            return Err(zbus::fdo::Error::Failed(
                "no account that could answer is known".to_string(),
            ));
        };
        let (answer, verdict) = async_channel::bounded(1);
        (self.tell)(FromAgent::Begin(Ask {
            message: message.to_string(),
            action: action_id.to_string(),
            user,
            cookie: cookie.to_string(),
            answer,
        }));
        match verdict.recv().await {
            Ok(true) => Ok(()),
            _ => Err(zbus::fdo::Error::Failed(
                "the request was dismissed".to_string(),
            )),
        }
    }

    fn cancel_authentication(&self, cookie: &str) {
        (self.tell)(FromAgent::Cancel(cookie.to_string()));
    }
}

fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

/// The name of an account, from the system's list of them.
pub fn user_name(uid: u32) -> Option<String> {
    let listed = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|text| name_in(&text, uid));
    listed.or_else(|| {
        // Accounts that are not in the file (a directory service).
        let output = Command::new("id")
            .args(["-nu", &uid.to_string()])
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !name.is_empty()).then_some(name)
    })
}

fn name_in(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let number = fields.nth(1)?.parse::<u32>().ok()?;
        (number == uid && !name.is_empty()).then(|| name.to_string())
    })
}

/// The login session this program belongs to, as the login service knows it.
fn session_id(connection: &zbus::blocking::Connection) -> Option<String> {
    if let Some(id) = std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|id| !id.is_empty())
    {
        return Some(id);
    }
    // A service of the user is in no session itself: the user's display
    // session is the one meant.
    let uid = own_uid()?;
    let user_path: zbus::zvariant::OwnedObjectPath = connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetUser",
            &(uid,),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let display: zbus::zvariant::OwnedValue = connection
        .call_method(
            Some("org.freedesktop.login1"),
            user_path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.User", "Display"),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let (id, _path): (String, zbus::zvariant::OwnedObjectPath) = display.try_into().ok()?;
    (!id.is_empty()).then_some(id)
}

/// Become the session's authentication agent. `tell` is called for what
/// polkit asks. An error when the system's bus or polkit is not there, or
/// the session already has an agent.
pub fn serve(
    tell: impl Fn(FromAgent) + Send + Sync + 'static,
) -> anyhow::Result<zbus::blocking::Connection> {
    let connection = zbus::blocking::connection::Builder::system()?
        .serve_at(AGENT_PATH, Agent { tell })?
        .build()?;
    let session =
        session_id(&connection).ok_or_else(|| anyhow::anyhow!("the login session is not known"))?;
    let mut details = std::collections::HashMap::new();
    details.insert("session-id", zbus::zvariant::Value::from(session.as_str()));
    let subject = ("unix-session", details);
    let locale = std::env::var("LANG")
        .ok()
        .filter(|locale| !locale.is_empty())
        .unwrap_or_else(|| "en_US.UTF-8".to_string());
    connection
        .call_method(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            Some("org.freedesktop.PolicyKit1.Authority"),
            "RegisterAuthenticationAgent",
            &(subject, locale.as_str(), AGENT_PATH),
        )
        .map_err(|error| {
            anyhow::anyhow!("polkit did not take the agent for session {session}: {error}")
        })?;
    log::info!("Authentication agent for session {session}");
    Ok(connection)
}

/// What the helper said at the end of its talk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Yes,
    /// No, with what the system said about it, if anything.
    No(String),
    /// The user took the request back while the system was asking.
    Refused,
    /// The helper could not be asked.
    Broken(String),
}

/// A question of the system's check as the dialog shows it: its usual
/// one for a password with whose password it is, any other as it is
/// (without the colon it ends with).
pub fn question_shown(question: &str, user: &str) -> String {
    let plain = question.trim().trim_end_matches(':').trim();
    if plain.is_empty() || plain.eq_ignore_ascii_case("password") {
        format!("Password of {user}")
    } else {
        plain.to_string()
    }
}

/// Where polkit's helper listens on systems that start it for whoever
/// connects (the helper needs no special rights of its own there).
pub const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Have the system check the user for a request, and wait for what it
/// says. The system leads the talk: whatever it asks for (a password, a
/// code, both one after the other) is asked of the user through `ask`,
/// which is given the system's question and whether what is typed may
/// be seen, and answers with what was typed (`None`: the user took the
/// request back); whatever it only says (a finger to be put on a
/// reader, a mistake) is passed to `say`. Blocks for as long as the
/// system and the user take: call it on a thread of its own.
///
/// The helper is reached through its socket where the system has one,
/// else by starting its program.
pub fn check(
    user: &str,
    cookie: &str,
    ask: impl FnMut(&str, bool) -> Option<String>,
    say: impl FnMut(&str),
) -> Verdict {
    if let Ok(socket) = std::os::unix::net::UnixStream::connect(HELPER_SOCKET) {
        let Ok(mut to_helper) = socket.try_clone() else {
            return Verdict::Broken("polkit's helper could not be spoken to".to_string());
        };
        // (Through the socket the helper is told the user first: a
        // program that is started is told it as its argument.)
        return converse(
            &mut to_helper,
            BufReader::new(socket),
            Some(user),
            cookie,
            ask,
            say,
        );
    }
    let Some(helper) = HELPERS
        .iter()
        .find(|path| std::path::Path::new(path).is_file())
    else {
        return Verdict::Broken("polkit's helper program was not found".to_string());
    };
    let child = Command::new(helper)
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return Verdict::Broken(format!("polkit's helper did not start: {error}")),
    };
    let (Some(mut to_helper), Some(from_helper)) = (child.stdin.take(), child.stdout.take()) else {
        return Verdict::Broken("polkit's helper has no pipes".to_string());
    };
    let said = converse(
        &mut to_helper,
        BufReader::new(from_helper),
        None,
        cookie,
        ask,
        say,
    );
    drop(to_helper);
    // (Taken back in the middle: the helper is not waited for.)
    if said == Verdict::Refused {
        let _ = child.kill();
    }
    let _ = child.wait();
    said
}

/// The talk with the helper: the user (`Some`: through its socket) and
/// the request's cookie go in first; then it relays what the system's
/// check asks and says, one line each, and ends with `SUCCESS` or
/// `FAILURE`.
pub fn converse(
    to_helper: &mut impl Write,
    from_helper: impl BufRead,
    user: Option<&str>,
    cookie: &str,
    mut ask: impl FnMut(&str, bool) -> Option<String>,
    mut say: impl FnMut(&str),
) -> Verdict {
    let begun = match user {
        Some(user) => writeln!(to_helper, "{user}"),
        None => Ok(()),
    }
    .and_then(|()| writeln!(to_helper, "{cookie}"))
    .and_then(|()| to_helper.flush());
    if begun.is_err() {
        return Verdict::Broken("polkit's helper did not listen".to_string());
    }
    let mut told = String::new();
    for line in from_helper.lines() {
        let Ok(line) = line else {
            break;
        };
        let line = line.trim_end();
        if line == "SUCCESS" {
            return Verdict::Yes;
        }
        if line == "FAILURE" {
            return Verdict::No(told);
        }
        let asked = line
            .strip_prefix("PAM_PROMPT_ECHO_OFF")
            .map(|question| (question, false))
            .or_else(|| {
                line.strip_prefix("PAM_PROMPT_ECHO_ON")
                    .map(|question| (question, true))
            });
        if let Some((question, seen)) = asked {
            let Some(mut answer) = ask(question.trim(), seen) else {
                return Verdict::Refused;
            };
            let sent = writeln!(to_helper, "{answer}").and_then(|()| to_helper.flush());
            // SAFETY: zero bytes are valid UTF-8; the length is unchanged.
            unsafe { answer.as_bytes_mut() }.fill(0);
            if sent.is_err() {
                return Verdict::Broken("polkit's helper went away".to_string());
            }
        } else if let Some(text) = line
            .strip_prefix("PAM_ERROR_MSG")
            .or_else(|| line.strip_prefix("PAM_TEXT_INFO"))
        {
            told = text.trim().to_string();
            say(&told);
            // It waits for an empty line before it goes on.
            let _ = writeln!(to_helper).and_then(|()| to_helper.flush());
        }
    }
    Verdict::No(told)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_talk_with_the_helper_ends_in_its_verdict() {
        let none = |_: &str| {};
        let mut sent = Vec::new();
        let said = converse(
            &mut sent,
            "PAM_PROMPT_ECHO_OFF Password: \nSUCCESS\n".as_bytes(),
            None,
            "cookie-1",
            |question, seen| {
                assert_eq!((question, seen), ("Password:", false));
                Some("secret".to_string())
            },
            none,
        );
        assert_eq!(said, Verdict::Yes);
        assert_eq!(String::from_utf8(sent).expect("text"), "cookie-1\nsecret\n");

        let mut sent = Vec::new();
        let mut heard = Vec::new();
        let said = converse(
            &mut sent,
            "PAM_PROMPT_ECHO_OFF Password: \nPAM_ERROR_MSG Sorry, try again.\nFAILURE\n".as_bytes(),
            None,
            "c",
            |_, _| Some("wrong".to_string()),
            |text| heard.push(text.to_string()),
        );
        assert_eq!(said, Verdict::No("Sorry, try again.".to_string()));
        assert_eq!(heard, ["Sorry, try again."]);
        // The message is answered with an empty line.
        assert_eq!(String::from_utf8(sent).expect("text"), "c\nwrong\n\n");

        // A helper that goes away without a word is a no.
        let mut sent = Vec::new();
        assert_eq!(
            converse(&mut sent, "".as_bytes(), None, "c", |_, _| None, none),
            Verdict::No(String::new())
        );
    }

    #[test]
    fn a_question_is_shown_in_the_systems_words() {
        assert_eq!(question_shown("Password: ", "ada"), "Password of ada");
        assert_eq!(question_shown("", "ada"), "Password of ada");
        assert_eq!(
            question_shown("Verification code:", "ada"),
            "Verification code"
        );
    }

    #[test]
    fn the_system_leads_the_talk() {
        // A finger first (only said), then a password and a code, each
        // asked for by its own question; the code may be seen.
        let script = "PAM_TEXT_INFO Place your finger on the reader\n\
                      PAM_PROMPT_ECHO_OFF Password: \n\
                      PAM_PROMPT_ECHO_ON Verification code: \n\
                      SUCCESS\n";
        let mut sent = Vec::new();
        let mut questions = Vec::new();
        let mut heard = Vec::new();
        let said = converse(
            &mut sent,
            script.as_bytes(),
            // (Through the socket: the user is told first.)
            Some("ada"),
            "cookie-7",
            |question, seen| {
                questions.push((question.to_string(), seen));
                Some(if seen { "123456" } else { "secret" }.to_string())
            },
            |text| heard.push(text.to_string()),
        );
        assert_eq!(said, Verdict::Yes);
        assert_eq!(heard, ["Place your finger on the reader"]);
        assert_eq!(
            questions,
            [
                ("Password:".to_string(), false),
                ("Verification code:".to_string(), true)
            ]
        );
        assert_eq!(
            String::from_utf8(sent).expect("text"),
            "ada\ncookie-7\n\nsecret\n123456\n"
        );
        // Taken back at a question: nothing more is sent.
        let mut sent = Vec::new();
        let said = converse(
            &mut sent,
            "PAM_PROMPT_ECHO_OFF Password: \nSUCCESS\n".as_bytes(),
            None,
            "c",
            |_, _| None,
            |_| {},
        );
        assert_eq!(said, Verdict::Refused);
        assert_eq!(String::from_utf8(sent).expect("text"), "c\n");
    }

    #[test]
    fn an_account_is_found_by_its_number() {
        let passwd = "root:x:0:0:Super User:/root:/bin/bash\ndaemon:x:2:2::/sbin:/usr/sbin/nologin\nada:x:1000:1000:Ada:/home/ada:/bin/bash\n";
        assert_eq!(name_in(passwd, 0).as_deref(), Some("root"));
        assert_eq!(name_in(passwd, 1000).as_deref(), Some("ada"));
        assert_eq!(name_in(passwd, 4242), None);
        assert_eq!(name_in("broken line\n::\n", 0), None);
    }
}
