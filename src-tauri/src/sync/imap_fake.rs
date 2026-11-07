//! An in-memory IMAP4rev1 server for tests.
//!
//! [`FakeImapServer`] implements [`MailTransport`], so an `ImapClient` built
//! with `.with_transport(server.transport())` runs every operation over the
//! real `imap` crate and its wire parsing, against a server whose state the
//! test can seed and inspect. It speaks the subset of IMAP the client sends:
//! `CAPABILITY`, `LOGIN`, `NOOP`, `LOGOUT`, `LIST` (incl. `RETURN
//! (SPECIAL-USE)`), `SELECT`, `EXAMINE`, `CREATE`, `RENAME`, `DELETE`,
//! `APPEND`, `EXPUNGE`, and the `UID` forms of `SEARCH`, `FETCH`, `STORE`,
//! `COPY`, `MOVE` and `EXPUNGE`.
//!
//! Capabilities are configurable, so server quirks (no `MOVE`, no `UIDPLUS`,
//! roles only on `RETURN (SPECIAL-USE)`) are one builder call away, and
//! [`FakeImapServer::fault`] injects failures: refused connections, throttled
//! logins, a connection dropped mid-operation, a command refused, or an
//! unsolicited untagged response interleaved before a reply.
//!
//! SMTP submissions are recorded ([`FakeImapServer::submitted`]).
//!
//! Test infrastructure only: compiled into test and debug builds.

use std::collections::{BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;

use crate::models::error::{AppError, Result};
use crate::sync::imap::ImapCredentials;
use crate::sync::imap_transport::{ImapStream, MailTransport};

/// One message as the server stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub uid: u32,
    pub flags: BTreeSet<String>,
    /// INTERNALDATE, epoch seconds.
    pub internal_date: i64,
    pub raw: Vec<u8>,
}

impl StoredMessage {
    /// The trimmed `Message-ID` header, when the message has one.
    pub fn message_id(&self) -> Option<String> {
        header_value(&self.raw, "Message-ID").map(|v| v.trim().to_string())
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }
}

#[derive(Debug, Clone)]
struct Folder {
    name: String,
    attributes: Vec<String>,
    uid_validity: u32,
    uid_next: u32,
    messages: Vec<StoredMessage>,
}

/// A failure the server produces on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The next `n` connection attempts fail before a stream opens.
    RefuseConnections(u32),
    /// The next `n` logins are answered `NO [UNAVAILABLE]` (throttling).
    ThrottleLogins(u32),
    /// Every login is rejected as bad credentials.
    RejectLogins,
    /// After `n` more commands (counted across connections), the connection
    /// that receives the next one closes without answering.
    DropAfterCommands(u32),
    /// The next command whose verb starts with `verb` (e.g. `"UID MOVE"`,
    /// `"SELECT"`) gets `reply` as its tagged answer, e.g.
    /// `"NO [SERVERBUG] Internal error"`.
    RefuseNext { verb: String, reply: String },
    /// Before answering the next command whose verb starts with `verb`, send
    /// the untagged `line` (without `* ` or CRLF), as when another client
    /// changes the mailbox at that moment.
    InterleaveNext { verb: String, line: String },
    /// SMTP submissions fail.
    RefuseSubmissions,
}

#[derive(Debug, Default)]
struct State {
    folders: Vec<Folder>,
    capabilities: Vec<String>,
    /// Role attributes (`\Sent`, `\Junk`…) appear in a plain `LIST` only when
    /// false; some servers send them only for `RETURN (SPECIAL-USE)`.
    roles_only_on_request: bool,
    password: Option<String>,
    faults: Vec<Fault>,
    commands: Vec<String>,
    connections: u32,
    next_uid_validity: u32,
    submitted: Vec<Vec<u8>>,
    clock: i64,
}

/// The default `INTERNALDATE` for appended messages that name none.
const DEFAULT_CLOCK: i64 = 1_793_000_000;

/// An in-memory IMAP server. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct FakeImapServer {
    state: Arc<Mutex<State>>,
}

impl Default for FakeImapServer {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeImapServer {
    /// A server with only `INBOX` and the `IMAP4rev1` capability.
    pub fn new() -> Self {
        let server = Self {
            state: Arc::new(Mutex::new(State {
                capabilities: vec!["IMAP4rev1".to_string()],
                next_uid_validity: 1000,
                clock: DEFAULT_CLOCK,
                ..State::default()
            })),
        };
        server.with_folder("INBOX", &[])
    }

    /// A typical modern server: `MOVE`, `UIDPLUS`, `SPECIAL-USE`, and the
    /// usual role folders.
    pub fn standard() -> Self {
        Self::new()
            .with_capabilities(&["IMAP4rev1", "MOVE", "UIDPLUS", "SPECIAL-USE"])
            .with_folder("Sent", &["\\Sent"])
            .with_folder("Drafts", &["\\Drafts"])
            .with_folder("Junk", &["\\Junk"])
            .with_folder("Trash", &["\\Trash"])
            .with_folder("Archive", &["\\Archive"])
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replace the advertised capabilities.
    pub fn with_capabilities(self, capabilities: &[&str]) -> Self {
        self.lock().capabilities = capabilities.iter().map(|c| (*c).to_string()).collect();
        self
    }

    /// Add a folder with the given `LIST` attributes.
    pub fn with_folder(self, name: &str, attributes: &[&str]) -> Self {
        let mut state = self.lock();
        let uid_validity = state.next_uid_validity;
        state.next_uid_validity += 1;
        state.folders.push(Folder {
            name: name.to_string(),
            attributes: attributes.iter().map(|a| (*a).to_string()).collect(),
            uid_validity,
            uid_next: 1,
            messages: Vec::new(),
        });
        drop(state);
        self
    }

    /// Send role attributes only for `LIST … RETURN (SPECIAL-USE)`.
    pub fn with_roles_only_on_request(self) -> Self {
        self.lock().roles_only_on_request = true;
        self
    }

    /// Accept only `password` at LOGIN (any password by default).
    pub fn with_password(self, password: &str) -> Self {
        self.lock().password = Some(password.to_string());
        self
    }

    /// Queue a fault. Faults of the same kind apply in the order added.
    pub fn fault(&self, fault: Fault) {
        self.lock().faults.push(fault);
    }

    /// The transport an `ImapClient` reaches this server through.
    pub fn transport(&self) -> Arc<dyn MailTransport> {
        Arc::new(self.clone())
    }

    /// File `raw` into `folder` as the server would on delivery; returns its UID.
    pub fn deliver(&self, folder: &str, raw: &[u8], flags: &[&str], internal_date: i64) -> u32 {
        let mut state = self.lock();
        let Some(target) = state.folders.iter_mut().find(|f| f.name == folder) else {
            panic!("FakeImapServer: no folder {folder}");
        };
        let uid = target.uid_next;
        target.uid_next += 1;
        target.messages.push(StoredMessage {
            uid,
            flags: flags.iter().map(|f| (*f).to_string()).collect(),
            internal_date,
            raw: raw.to_vec(),
        });
        uid
    }

    /// The messages of `folder`, in UID order. Empty for a missing folder.
    pub fn messages(&self, folder: &str) -> Vec<StoredMessage> {
        self.lock()
            .folders
            .iter()
            .find(|f| f.name == folder)
            .map(|f| f.messages.clone())
            .unwrap_or_default()
    }

    /// The folder holding the message with `message_id`, and its UID there.
    pub fn find(&self, message_id: &str) -> Option<(String, u32)> {
        self.lock().folders.iter().find_map(|f| {
            f.messages
                .iter()
                .find(|m| m.message_id().as_deref() == Some(message_id))
                .map(|m| (f.name.clone(), m.uid))
        })
    }

    pub fn folder_names(&self) -> Vec<String> {
        self.lock().folders.iter().map(|f| f.name.clone()).collect()
    }

    pub fn uid_validity(&self, folder: &str) -> Option<u32> {
        self.lock()
            .folders
            .iter()
            .find(|f| f.name == folder)
            .map(|f| f.uid_validity)
    }

    /// Rebuild `folder` the way a server migration does: a new UIDVALIDITY
    /// and every message renumbered from 1.
    pub fn renumber(&self, folder: &str) {
        let mut state = self.lock();
        let uid_validity = state.next_uid_validity;
        state.next_uid_validity += 1;
        if let Some(target) = state.folders.iter_mut().find(|f| f.name == folder) {
            target.uid_validity = uid_validity;
            for (i, message) in target.messages.iter_mut().enumerate() {
                message.uid = i as u32 + 1;
            }
            target.uid_next = target.messages.len() as u32 + 1;
        }
    }

    /// Every command received, without its tag, in order.
    pub fn commands(&self) -> Vec<String> {
        self.lock().commands.clone()
    }

    /// How many connections were opened.
    pub fn connections(&self) -> u32 {
        self.lock().connections
    }

    /// Raw RFC 5322 bytes of every SMTP submission.
    pub fn submitted(&self) -> Vec<Vec<u8>> {
        self.lock().submitted.clone()
    }

    fn take_fault(&self, pick: impl Fn(&Fault) -> bool) -> Option<Fault> {
        let mut state = self.lock();
        let index = state.faults.iter().position(pick)?;
        Some(state.faults.remove(index))
    }

    /// Decrement a counted fault; true when it fired.
    fn consume_counted(&self, kind: fn(&Fault) -> Option<u32>, rebuild: fn(u32) -> Fault) -> bool {
        let mut state = self.lock();
        let Some(index) = state.faults.iter().position(|f| kind(f).is_some()) else {
            return false;
        };
        let remaining = kind(&state.faults[index]).unwrap_or(0);
        if remaining <= 1 {
            state.faults.remove(index);
        } else {
            state.faults[index] = rebuild(remaining - 1);
        }
        remaining >= 1
    }

    pub fn connect(&self) -> std::result::Result<FakeImapConnection, imap::Error> {
        let refused = self.consume_counted(
            |f| match f {
                Fault::RefuseConnections(n) => Some(*n),
                _ => None,
            },
            Fault::RefuseConnections,
        );
        if refused {
            return Err(imap::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "fake server refused the connection",
            )));
        }
        self.lock().connections += 1;
        let mut connection = FakeImapConnection {
            server: self.clone(),
            selected: None,
            input: Vec::new(),
            output: VecDeque::new(),
            pending_append: None,
            closed: false,
        };
        connection.send("* OK [CAPABILITY IMAP4rev1] Fake IMAP ready");
        Ok(connection)
    }
}

#[async_trait]
impl MailTransport for FakeImapServer {
    fn open_imap(&self, _creds: &ImapCredentials) -> std::result::Result<Box<dyn ImapStream>, imap::Error> {
        Ok(Box::new(self.connect()?))
    }

    async fn send_smtp(&self, _creds: &ImapCredentials, email: lettre::Message) -> Result<()> {
        if self.take_fault(|f| *f == Fault::RefuseSubmissions).is_some() {
            self.fault(Fault::RefuseSubmissions);
            return Err(AppError::SyncError(
                "SMTP send failed: fake server refused the submission".to_string(),
            ));
        }
        self.lock().submitted.push(email.formatted());
        Ok(())
    }

    async fn test_smtp(&self, _creds: &ImapCredentials) -> Result<()> {
        Ok(())
    }
}

struct PendingAppend {
    tag: String,
    folder: String,
    flags: Vec<String>,
    internal_date: Option<i64>,
    remaining: usize,
    data: Vec<u8>,
}

/// One client connection to a [`FakeImapServer`]. Replies are produced as
/// each command line is written, so reads never block.
pub struct FakeImapConnection {
    server: FakeImapServer,
    /// Selected folder name and whether it is read-only (EXAMINE).
    selected: Option<(String, bool)>,
    input: Vec<u8>,
    output: VecDeque<u8>,
    pending_append: Option<PendingAppend>,
    closed: bool,
}

impl Read for FakeImapConnection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.output.len());
        for (slot, byte) in buf.iter_mut().zip(self.output.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
}

impl Write for FakeImapConnection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.closed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fake server closed the connection",
            ));
        }
        self.input.extend_from_slice(buf);
        self.process_input();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl FakeImapConnection {
    fn send(&mut self, line: &str) {
        self.output.extend(line.as_bytes());
        self.output.extend(b"\r\n");
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        self.output.extend(bytes);
    }

    fn process_input(&mut self) {
        loop {
            if self.closed {
                self.input.clear();
                return;
            }
            if let Some(pending) = self.pending_append.as_mut() {
                if pending.remaining > 0 {
                    let take = pending.remaining.min(self.input.len());
                    pending.data.extend(self.input.drain(..take));
                    pending.remaining -= take;
                    if pending.remaining > 0 {
                        return;
                    }
                }
                // The literal is followed by the CRLF that ends the command.
                let Some(end) = find_crlf(&self.input) else {
                    return;
                };
                self.input.drain(..end + 2);
                if let Some(pending) = self.pending_append.take() {
                    self.finish_append(pending);
                }
                continue;
            }
            let Some(end) = find_crlf(&self.input) else {
                return;
            };
            let line: Vec<u8> = self.input.drain(..end + 2).take(end).collect();
            let line = String::from_utf8_lossy(&line).into_owned();
            self.handle_line(&line);
        }
    }

    fn handle_line(&mut self, line: &str) {
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        let tag = tag.to_string();
        let tokens = tokenize(rest);
        let mut words = tokens.iter().map(Token::text);
        let first = words.next().unwrap_or_default().to_ascii_uppercase();
        let verb = if first == "UID" {
            format!("UID {}", words.next().unwrap_or_default().to_ascii_uppercase())
        } else {
            first
        };
        let args: Vec<Token> = tokens
            .into_iter()
            .skip(if verb.starts_with("UID ") { 2 } else { 1 })
            .collect();

        {
            let mut state = self.server.lock();
            state.commands.push(redact_login(rest));
            if let Some(index) = state
                .faults
                .iter()
                .position(|f| matches!(f, Fault::DropAfterCommands(_)))
            {
                if let Fault::DropAfterCommands(n) = state.faults[index] {
                    if n == 0 {
                        state.faults.remove(index);
                        drop(state);
                        self.closed = true;
                        return;
                    }
                    state.faults[index] = Fault::DropAfterCommands(n - 1);
                }
            }
        }
        if let Some(Fault::InterleaveNext { line, .. }) = self
            .server
            .take_fault(|f| matches!(f, Fault::InterleaveNext { verb: v, .. } if verb.starts_with(v.as_str())))
        {
            self.send(&format!("* {line}"));
        }
        if let Some(Fault::RefuseNext { reply, .. }) = self
            .server
            .take_fault(|f| matches!(f, Fault::RefuseNext { verb: v, .. } if verb.starts_with(v.as_str())))
        {
            self.send(&format!("{tag} {reply}"));
            return;
        }

        match verb.as_str() {
            "CAPABILITY" => {
                let caps = self.server.lock().capabilities.join(" ");
                self.send(&format!("* CAPABILITY {caps}"));
                self.ok(&tag, "CAPABILITY completed");
            }
            "LOGIN" => self.login(&tag, &args),
            "NOOP" | "CHECK" => self.ok(&tag, "NOOP completed"),
            "LOGOUT" => {
                self.send("* BYE Logging out");
                self.ok(&tag, "LOGOUT completed");
                self.closed = true;
            }
            "LIST" => self.list(&tag, &args),
            "SELECT" => self.select(&tag, &args, false),
            "EXAMINE" => self.select(&tag, &args, true),
            "CREATE" => self.create(&tag, &args),
            "RENAME" => self.rename(&tag, &args),
            "DELETE" => self.delete(&tag, &args),
            "APPEND" => self.append(&tag, &args),
            "EXPUNGE" => self.expunge(&tag, None),
            "UID SEARCH" => self.search(&tag, &args),
            "UID FETCH" => self.fetch(&tag, &args),
            "UID STORE" => self.store(&tag, &args),
            "UID COPY" => self.copy_or_move(&tag, &args, false),
            "UID MOVE" => self.copy_or_move(&tag, &args, true),
            "UID EXPUNGE" => {
                let set = args.first().map(Token::text).unwrap_or_default();
                self.expunge(&tag, Some(set));
            }
            _ => self.send(&format!("{tag} BAD Unknown command")),
        }
    }

    fn ok(&mut self, tag: &str, text: &str) {
        self.send(&format!("{tag} OK {text}"));
    }

    fn no(&mut self, tag: &str, text: &str) {
        self.send(&format!("{tag} NO {text}"));
    }

    fn login(&mut self, tag: &str, args: &[Token]) {
        let password = args.get(1).map(Token::text).unwrap_or_default();
        if self.server.take_fault(|f| *f == Fault::RejectLogins).is_some() {
            self.server.fault(Fault::RejectLogins);
            return self.no(tag, "[AUTHENTICATIONFAILED] Invalid credentials");
        }
        let throttled = self.server.consume_counted(
            |f| match f {
                Fault::ThrottleLogins(n) => Some(*n),
                _ => None,
            },
            Fault::ThrottleLogins,
        );
        if throttled {
            return self.no(tag, "[UNAVAILABLE] Too many logins, try again later");
        }
        let expected = self.server.lock().password.clone();
        if expected.is_some_and(|p| p != password) {
            return self.no(tag, "[AUTHENTICATIONFAILED] Invalid credentials");
        }
        self.ok(tag, "Logged in");
    }

    fn list(&mut self, tag: &str, args: &[Token]) {
        let pattern = args.get(1).map(Token::text).unwrap_or_default();
        let wants_roles = args.iter().any(
            |a| matches!(a, Token::List(items) if items.iter().any(|i| i.text().eq_ignore_ascii_case("SPECIAL-USE"))),
        );
        let state = self.server.lock();
        let show_roles = wants_roles || !state.roles_only_on_request;
        let lines: Vec<String> = state
            .folders
            .iter()
            .filter(|f| pattern.contains('*') || pattern.contains('%') || f.name == pattern)
            .map(|f| {
                let mut attributes: Vec<&str> = vec!["\\HasNoChildren"];
                if show_roles {
                    attributes.extend(f.attributes.iter().map(String::as_str));
                }
                format!("* LIST ({}) \"/\" {}", attributes.join(" "), quote(&f.name))
            })
            .collect();
        drop(state);
        for line in lines {
            self.send(&line);
        }
        self.ok(tag, "LIST completed");
    }

    fn select(&mut self, tag: &str, args: &[Token], read_only: bool) {
        let name = args.first().map(Token::text).unwrap_or_default();
        let state = self.server.lock();
        let Some(folder) = state
            .folders
            .iter()
            .find(|f| f.name == name || (f.name == "INBOX" && name.eq_ignore_ascii_case("INBOX")))
        else {
            drop(state);
            self.selected = None;
            return self.no(tag, "[NONEXISTENT] Mailbox doesn't exist");
        };
        let lines = [
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)".to_string(),
            format!("* {} EXISTS", folder.messages.len()),
            "* 0 RECENT".to_string(),
            format!("* OK [UIDVALIDITY {}] UIDs valid", folder.uid_validity),
            format!("* OK [UIDNEXT {}] Predicted next UID", folder.uid_next),
        ];
        let folder_name = folder.name.clone();
        drop(state);
        for line in lines {
            self.send(&line);
        }
        self.selected = Some((folder_name, read_only));
        let mode = if read_only { "READ-ONLY" } else { "READ-WRITE" };
        self.ok(tag, &format!("[{mode}] SELECT completed"));
    }

    fn create(&mut self, tag: &str, args: &[Token]) {
        let name = args.first().map(Token::text).unwrap_or_default();
        if self.server.lock().folders.iter().any(|f| f.name == name) {
            return self.no(tag, "[ALREADYEXISTS] Mailbox exists");
        }
        let _ = self.server.clone().with_folder(&name, &[]);
        self.ok(tag, "CREATE completed");
    }

    fn rename(&mut self, tag: &str, args: &[Token]) {
        let old = args.first().map(Token::text).unwrap_or_default();
        let new = args.get(1).map(Token::text).unwrap_or_default();
        let mut state = self.server.lock();
        if state.folders.iter().any(|f| f.name == new) {
            drop(state);
            return self.no(tag, "[ALREADYEXISTS] Mailbox exists");
        }
        match state.folders.iter_mut().find(|f| f.name == old) {
            Some(folder) => {
                folder.name = new;
                drop(state);
                self.ok(tag, "RENAME completed");
            }
            None => {
                drop(state);
                self.no(tag, "[NONEXISTENT] Mailbox doesn't exist");
            }
        }
    }

    fn delete(&mut self, tag: &str, args: &[Token]) {
        let name = args.first().map(Token::text).unwrap_or_default();
        let mut state = self.server.lock();
        let before = state.folders.len();
        state.folders.retain(|f| f.name != name);
        let removed = state.folders.len() < before;
        drop(state);
        if removed {
            self.ok(tag, "DELETE completed");
        } else {
            self.no(tag, "[NONEXISTENT] Mailbox doesn't exist");
        }
    }

    fn append(&mut self, tag: &str, args: &[Token]) {
        let folder = args.first().map(Token::text).unwrap_or_default();
        let flags = args
            .iter()
            .find_map(|a| match a {
                Token::List(items) => Some(items.iter().map(Token::text).collect()),
                _ => None,
            })
            .unwrap_or_default();
        let internal_date = args.iter().skip(1).find_map(|a| match a {
            Token::Quoted(s) => parse_internal_date(s),
            _ => None,
        });
        let Some(Token::Literal(size)) = args.last() else {
            return self.send(&format!("{tag} BAD APPEND needs a literal"));
        };
        if !self.server.lock().folders.iter().any(|f| f.name == folder) {
            return self.no(tag, "[TRYCREATE] Mailbox doesn't exist");
        }
        self.pending_append = Some(PendingAppend {
            tag: tag.to_string(),
            folder,
            flags,
            internal_date,
            remaining: *size,
            data: Vec::new(),
        });
        self.send("+ Ready for literal data");
    }

    fn finish_append(&mut self, pending: PendingAppend) {
        let mut state = self.server.lock();
        let clock = state.clock;
        let Some(folder) = state.folders.iter_mut().find(|f| f.name == pending.folder) else {
            drop(state);
            return self.no(&pending.tag, "[TRYCREATE] Mailbox doesn't exist");
        };
        let uid = folder.uid_next;
        folder.uid_next += 1;
        folder.messages.push(StoredMessage {
            uid,
            flags: pending.flags.into_iter().collect(),
            internal_date: pending.internal_date.unwrap_or(clock),
            raw: pending.data,
        });
        let uid_validity = folder.uid_validity;
        drop(state);
        self.ok(
            &pending.tag,
            &format!("[APPENDUID {uid_validity} {uid}] APPEND completed"),
        );
    }

    /// The selected folder, or a `BAD` reply when nothing is selected.
    fn selected_or_bad(&mut self, tag: &str) -> Option<(String, bool)> {
        let selected = self.selected.clone();
        if selected.is_none() {
            self.send(&format!("{tag} BAD No mailbox selected"));
        }
        selected
    }

    fn search(&mut self, tag: &str, args: &[Token]) {
        let Some((folder, _)) = self.selected_or_bad(tag) else {
            return;
        };
        let criteria = match parse_search(args) {
            Ok(criteria) => criteria,
            Err(e) => return self.send(&format!("{tag} BAD {e}")),
        };
        let state = self.server.lock();
        let hits: Vec<String> = state
            .folders
            .iter()
            .find(|f| f.name == folder)
            .map(|f| {
                f.messages
                    .iter()
                    .filter(|m| criteria.iter().all(|c| c.matches(m)))
                    .map(|m| m.uid.to_string())
                    .collect()
            })
            .unwrap_or_default();
        drop(state);
        if hits.is_empty() {
            self.send("* SEARCH");
        } else {
            self.send(&format!("* SEARCH {}", hits.join(" ")));
        }
        self.ok(tag, "SEARCH completed");
    }

    fn fetch(&mut self, tag: &str, args: &[Token]) {
        let Some((folder, read_only)) = self.selected_or_bad(tag) else {
            return;
        };
        let set = args.first().map(Token::text).unwrap_or_default();
        let items: Vec<String> = match args.get(1) {
            Some(Token::List(items)) => items.iter().map(Token::text).collect(),
            Some(single) => vec![single.text()],
            None => Vec::new(),
        };
        let mut replies: Vec<Vec<u8>> = Vec::new();
        {
            let mut state = self.server.lock();
            let Some(target) = state.folders.iter_mut().find(|f| f.name == folder) else {
                drop(state);
                return self.no(tag, "[NONEXISTENT] Mailbox is gone");
            };
            let max_uid = target.messages.iter().map(|m| m.uid).max().unwrap_or(0);
            for (index, message) in target.messages.iter_mut().enumerate() {
                if !uid_in_set(message.uid, &set, max_uid) {
                    continue;
                }
                let mut parts: Vec<Vec<u8>> = vec![format!("UID {}", message.uid).into_bytes()];
                for item in &items {
                    let upper = item.to_ascii_uppercase();
                    if upper == "UID" {
                        continue;
                    }
                    if upper == "FLAGS" {
                        parts.push(format!("FLAGS ({})", join_flags(&message.flags)).into_bytes());
                    } else if upper == "INTERNALDATE" {
                        parts.push(
                            format!("INTERNALDATE \"{}\"", format_internal_date(message.internal_date)).into_bytes(),
                        );
                    } else if upper == "BODY.PEEK[]" || upper == "BODY[]" || upper == "RFC822" {
                        let name = if upper == "RFC822" { "RFC822" } else { "BODY[]" };
                        if !upper.contains("PEEK") && !read_only {
                            message.flags.insert("\\Seen".to_string());
                        }
                        let mut part = format!("{name} {{{}}}\r\n", message.raw.len()).into_bytes();
                        part.extend_from_slice(&message.raw);
                        parts.push(part);
                    } else if let Some(fields) = header_fields_item(&upper) {
                        let header = header_subset(&message.raw, &fields);
                        let mut part =
                            format!("BODY[HEADER.FIELDS ({})] {{{}}}\r\n", fields.join(" "), header.len()).into_bytes();
                        part.extend_from_slice(&header);
                        parts.push(part);
                    }
                }
                let mut line = format!("* {} FETCH (", index + 1).into_bytes();
                line.extend(parts.join(&b' '));
                line.extend_from_slice(b")\r\n");
                replies.push(line);
            }
        }
        for reply in replies {
            self.send_bytes(&reply);
        }
        self.ok(tag, "FETCH completed");
    }

    fn store(&mut self, tag: &str, args: &[Token]) {
        let Some((folder, read_only)) = self.selected_or_bad(tag) else {
            return;
        };
        if read_only {
            return self.no(tag, "Mailbox is read-only");
        }
        let set = args.first().map(Token::text).unwrap_or_default();
        let mode = args.get(1).map(Token::text).unwrap_or_default().to_ascii_uppercase();
        let flags: Vec<String> = match args.get(2) {
            Some(Token::List(items)) => items.iter().map(Token::text).collect(),
            Some(single) => vec![single.text()],
            None => Vec::new(),
        };
        let silent = mode.ends_with(".SILENT");
        let mut echoes = Vec::new();
        {
            let mut state = self.server.lock();
            if let Some(target) = state.folders.iter_mut().find(|f| f.name == folder) {
                let max_uid = target.messages.iter().map(|m| m.uid).max().unwrap_or(0);
                for (index, message) in target.messages.iter_mut().enumerate() {
                    if !uid_in_set(message.uid, &set, max_uid) {
                        continue;
                    }
                    if mode.starts_with('+') {
                        message.flags.extend(flags.iter().cloned());
                    } else if mode.starts_with('-') {
                        message
                            .flags
                            .retain(|f| !flags.iter().any(|g| g.eq_ignore_ascii_case(f)));
                    } else {
                        message.flags = flags.iter().cloned().collect();
                    }
                    if !silent {
                        echoes.push(format!(
                            "* {} FETCH (UID {} FLAGS ({}))",
                            index + 1,
                            message.uid,
                            join_flags(&message.flags)
                        ));
                    }
                }
            }
        }
        for echo in echoes {
            self.send(&echo);
        }
        self.ok(tag, "STORE completed");
    }

    fn copy_or_move(&mut self, tag: &str, args: &[Token], is_move: bool) {
        let Some((folder, _)) = self.selected_or_bad(tag) else {
            return;
        };
        let set = args.first().map(Token::text).unwrap_or_default();
        let target_name = args.get(1).map(Token::text).unwrap_or_default();
        let mut untagged = Vec::new();
        let code;
        {
            let mut state = self.server.lock();
            let Some(target_index) = state.folders.iter().position(|f| f.name == target_name) else {
                drop(state);
                return self.no(tag, "[TRYCREATE] Mailbox doesn't exist");
            };
            let Some(source_index) = state.folders.iter().position(|f| f.name == folder) else {
                drop(state);
                return self.no(tag, "[NONEXISTENT] Mailbox is gone");
            };
            let max_uid = state.folders[source_index]
                .messages
                .iter()
                .map(|m| m.uid)
                .max()
                .unwrap_or(0);
            let picked: Vec<(usize, StoredMessage)> = state.folders[source_index]
                .messages
                .iter()
                .enumerate()
                .filter(|(_, m)| uid_in_set(m.uid, &set, max_uid))
                .map(|(i, m)| (i, m.clone()))
                .collect();
            let mut source_uids = Vec::new();
            let mut target_uids = Vec::new();
            for (_, message) in &picked {
                let target = &mut state.folders[target_index];
                let uid = target.uid_next;
                target.uid_next += 1;
                let mut copy = message.clone();
                copy.uid = uid;
                copy.flags.remove("\\Deleted");
                target.messages.push(copy);
                source_uids.push(message.uid.to_string());
                target_uids.push(uid.to_string());
            }
            let uid_validity = state.folders[target_index].uid_validity;
            code = if picked.is_empty() {
                String::new()
            } else {
                format!(
                    "[COPYUID {uid_validity} {} {}] ",
                    source_uids.join(","),
                    target_uids.join(",")
                )
            };
            if is_move {
                // Expunge in descending sequence order so each number stays valid.
                for (index, message) in picked.iter().rev() {
                    let source = &mut state.folders[source_index];
                    source.messages.retain(|m| m.uid != message.uid);
                    untagged.push(format!("* {} EXPUNGE", index + 1));
                }
            }
        }
        if is_move {
            if !code.is_empty() {
                self.send(&format!("* OK {}Moved", code));
            }
            for line in untagged {
                self.send(&line);
            }
            self.ok(tag, "MOVE completed");
        } else {
            self.ok(tag, &format!("{code}COPY completed"));
        }
    }

    fn expunge(&mut self, tag: &str, uid_set: Option<String>) {
        let Some((folder, read_only)) = self.selected_or_bad(tag) else {
            return;
        };
        if read_only {
            return self.no(tag, "Mailbox is read-only");
        }
        let mut lines = Vec::new();
        {
            let mut state = self.server.lock();
            if let Some(target) = state.folders.iter_mut().find(|f| f.name == folder) {
                let max_uid = target.messages.iter().map(|m| m.uid).max().unwrap_or(0);
                let mut index = 0;
                while index < target.messages.len() {
                    let message = &target.messages[index];
                    let in_set = uid_set
                        .as_deref()
                        .is_none_or(|set| uid_in_set(message.uid, set, max_uid));
                    if in_set && message.has_flag("\\Deleted") {
                        target.messages.remove(index);
                        lines.push(format!("* {} EXPUNGE", index + 1));
                    } else {
                        index += 1;
                    }
                }
            }
        }
        for line in lines {
            self.send(&line);
        }
        self.ok(tag, "EXPUNGE completed");
    }
}

// ── Wire helpers ────────────────────────────────────────────────────────────

fn find_crlf(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|w| w == b"\r\n")
}

fn redact_login(command: &str) -> String {
    if command.len() >= 6 && command[..6].eq_ignore_ascii_case("LOGIN ") {
        "LOGIN ***".to_string()
    } else {
        command.to_string()
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn join_flags(flags: &BTreeSet<String>) -> String {
    flags.iter().map(String::as_str).collect::<Vec<_>>().join(" ")
}

/// One argument of a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Atom(String),
    Quoted(String),
    List(Vec<Token>),
    /// `{n}` at the end of a line: n bytes follow.
    Literal(usize),
}

impl Token {
    fn text(&self) -> String {
        match self {
            Self::Atom(s) | Self::Quoted(s) => s.clone(),
            Self::List(items) => format!("({})", items.iter().map(Token::text).collect::<Vec<_>>().join(" ")),
            Self::Literal(n) => format!("{{{n}}}"),
        }
    }
}

fn tokenize(line: &str) -> Vec<Token> {
    let chars: Vec<char> = line.chars().collect();
    let mut pos = 0;
    tokenize_until(&chars, &mut pos, None)
}

fn tokenize_until(chars: &[char], pos: &mut usize, close: Option<char>) -> Vec<Token> {
    let mut tokens = Vec::new();
    while *pos < chars.len() {
        let c = chars[*pos];
        if c == ' ' {
            *pos += 1;
        } else if Some(c) == close {
            *pos += 1;
            return tokens;
        } else if c == '(' {
            *pos += 1;
            tokens.push(Token::List(tokenize_until(chars, pos, Some(')'))));
        } else if c == '"' {
            *pos += 1;
            let mut s = String::new();
            while *pos < chars.len() && chars[*pos] != '"' {
                if chars[*pos] == '\\' && *pos + 1 < chars.len() {
                    *pos += 1;
                }
                s.push(chars[*pos]);
                *pos += 1;
            }
            *pos += 1;
            tokens.push(Token::Quoted(s));
        } else if c == '{' {
            let rest: String = chars[*pos + 1..].iter().collect();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            *pos += digits.len() + 2;
            tokens.push(Token::Literal(digits.parse().unwrap_or(0)));
        } else {
            // An atom; a `[...]` section inside it (BODY.PEEK[HEADER.FIELDS
            // (MESSAGE-ID)]) may contain spaces and parentheses.
            let mut s = String::new();
            let mut depth = 0;
            while *pos < chars.len() {
                let ch = chars[*pos];
                if depth == 0 && (ch == ' ' || ch == '(' || Some(ch) == close) {
                    break;
                }
                if ch == '[' {
                    depth += 1;
                } else if ch == ']' {
                    depth -= 1;
                }
                s.push(ch);
                *pos += 1;
            }
            tokens.push(Token::Atom(s));
        }
    }
    tokens
}

/// Whether `uid` is in an IMAP sequence set like `1:*`, `4`, `2,5:7`.
fn uid_in_set(uid: u32, set: &str, max_uid: u32) -> bool {
    let bound = |s: &str| if s == "*" { Some(max_uid) } else { s.parse::<u32>().ok() };
    set.split(',').any(|range| match range.split_once(':') {
        Some((a, b)) => match (bound(a), bound(b)) {
            (Some(a), Some(b)) => (a.min(b)..=a.max(b)).contains(&uid),
            _ => false,
        },
        None => bound(range) == Some(uid),
    })
}

/// The field names of a `BODY.PEEK[HEADER.FIELDS (A B)]` item.
fn header_fields_item(upper: &str) -> Option<Vec<String>> {
    let inner = upper
        .strip_prefix("BODY.PEEK[HEADER.FIELDS (")
        .or_else(|| upper.strip_prefix("BODY[HEADER.FIELDS ("))?;
    let fields = inner.split(')').next()?;
    Some(fields.split_whitespace().map(str::to_string).collect())
}

/// The header block of `raw`, as (name, unfolded value) pairs.
fn headers(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in head.split("\r\n") {
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = out.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
        } else if let Some((name, value)) = line.split_once(':') {
            out.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    out
}

fn header_value(raw: &[u8], name: &str) -> Option<String> {
    headers(raw)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn header_subset(raw: &[u8], fields: &[String]) -> Vec<u8> {
    let mut out = String::new();
    for (name, value) in headers(raw) {
        if fields.iter().any(|f| f.eq_ignore_ascii_case(&name)) {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    out.push_str("\r\n");
    out.into_bytes()
}

fn format_internal_date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .format("%d-%b-%Y %H:%M:%S +0000")
        .to_string()
}

fn parse_internal_date(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_str(s.trim(), "%d-%b-%Y %H:%M:%S %z")
        .ok()
        .map(|d| d.timestamp())
}

/// `d-Mon-yyyy` as the epoch second the day starts (UTC).
fn parse_search_date(s: &str) -> Option<i64> {
    chrono::NaiveDate::parse_from_str(s, "%d-%b-%Y")
        .ok()?
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().timestamp())
}

/// One `SEARCH` key.
#[derive(Debug)]
enum Criterion {
    All,
    Uid(String),
    Header(String, String),
    Since(i64),
    Before(i64),
    Flag(String, bool),
    Not(Box<Criterion>),
    Or(Box<Criterion>, Box<Criterion>),
    And(Vec<Criterion>),
}

impl Criterion {
    fn matches(&self, message: &StoredMessage) -> bool {
        match self {
            Self::All => true,
            Self::Uid(set) => uid_in_set(message.uid, set, u32::MAX),
            Self::Header(name, needle) => header_value(&message.raw, name)
                .is_some_and(|v| v.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())),
            Self::Since(day) => message.internal_date >= *day,
            Self::Before(day) => message.internal_date < *day,
            Self::Flag(flag, wanted) => message.has_flag(flag) == *wanted,
            Self::Not(inner) => !inner.matches(message),
            Self::Or(a, b) => a.matches(message) || b.matches(message),
            Self::And(all) => all.iter().all(|c| c.matches(message)),
        }
    }
}

fn parse_search(args: &[Token]) -> std::result::Result<Vec<Criterion>, String> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < args.len() {
        out.push(parse_criterion(args, &mut pos)?);
    }
    Ok(out)
}

fn parse_criterion(args: &[Token], pos: &mut usize) -> std::result::Result<Criterion, String> {
    let token = args.get(*pos).ok_or("missing search key")?.clone();
    *pos += 1;
    let next = |pos: &mut usize| -> std::result::Result<String, String> {
        let value = args.get(*pos).map(Token::text).ok_or("missing search argument")?;
        *pos += 1;
        Ok(value)
    };
    if let Token::List(items) = token {
        return Ok(Criterion::And(parse_search(&items)?));
    }
    let key = token.text().to_ascii_uppercase();
    Ok(match key.as_str() {
        "ALL" => Criterion::All,
        "UID" => Criterion::Uid(next(pos)?),
        "HEADER" => {
            let name = next(pos)?;
            Criterion::Header(name, next(pos)?)
        }
        "SINCE" => Criterion::Since(parse_search_date(&next(pos)?).ok_or("bad date")?),
        "BEFORE" => Criterion::Before(parse_search_date(&next(pos)?).ok_or("bad date")?),
        "SEEN" => Criterion::Flag("\\Seen".to_string(), true),
        "UNSEEN" => Criterion::Flag("\\Seen".to_string(), false),
        "FLAGGED" => Criterion::Flag("\\Flagged".to_string(), true),
        "UNFLAGGED" => Criterion::Flag("\\Flagged".to_string(), false),
        "DELETED" => Criterion::Flag("\\Deleted".to_string(), true),
        "UNDELETED" => Criterion::Flag("\\Deleted".to_string(), false),
        "NOT" => Criterion::Not(Box::new(parse_criterion(args, pos)?)),
        "OR" => {
            let a = parse_criterion(args, pos)?;
            let b = parse_criterion(args, pos)?;
            Criterion::Or(Box::new(a), Box::new(b))
        }
        other => return Err(format!("unsupported search key {other}")),
    })
}

/// A minimal RFC 5322 message for seeding a fake mailbox. Synthetic data only.
pub fn synthetic_message(message_id: &str, subject: &str, from: &str, to: &str, date: i64) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp(date, 0)
        .unwrap_or_default()
        .to_rfc2822();
    format!(
        "Message-ID: {message_id}\r\n\
         Date: {date}\r\n\
         From: {from}\r\n\
         To: {to}\r\n\
         Subject: {subject}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         Body of {subject}.\r\n"
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_with_spaces_stays_one_atom() {
        let tokens = tokenize("1:* (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])");
        assert_eq!(
            tokens,
            vec![
                Token::Atom("1:*".into()),
                Token::List(vec![
                    Token::Atom("UID".into()),
                    Token::Atom("INTERNALDATE".into()),
                    Token::Atom("BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]".into()),
                ]),
            ]
        );
    }

    #[test]
    fn quoted_strings_unescape() {
        assert_eq!(tokenize(r#""a \"b\"""#), vec![Token::Quoted("a \"b\"".into())]);
    }

    #[test]
    fn sequence_sets_cover_ranges_lists_and_star() {
        assert!(uid_in_set(5, "1:*", 9));
        assert!(uid_in_set(9, "7:*", 9));
        assert!(!uid_in_set(6, "1,3,7:8", 9));
        assert!(uid_in_set(7, "1,3,7:8", 9));
    }

    #[test]
    fn or_and_header_criteria_combine() {
        let raw = synthetic_message("<a@example.test>", "Hi", "x@example.test", "y@example.test", 0);
        let message = StoredMessage {
            uid: 1,
            flags: BTreeSet::new(),
            internal_date: DEFAULT_CLOCK,
            raw,
        };
        let criteria = parse_search(&tokenize(
            r#"OR HEADER Content-Type "multipart/mixed" HEADER Message-ID "<a@example.test>""#,
        ))
        .expect("parse");
        assert!(criteria.iter().all(|c| c.matches(&message)));
    }
}

// ── Transcript replay ───────────────────────────────────────────────────────

/// A server that replays a recorded IMAP conversation, for responses neither
/// [`FakeImapServer`] nor a real test server produce on demand (folder names
/// sent as literals, odd FETCH item order, `* BYE` mid-command, `[ALERT]`…).
///
/// Transcript format, one connection per `=== connection` section (the first
/// marker may be omitted):
///
/// ```text
/// # comment
/// S: * OK greeting            server line; `{tag}` is the current command's tag
/// C: LOGIN *                  the next command, without its tag; a trailing `*`
///                             matches any rest of the line
/// S: * 1 FETCH (BODY[] {=}    `{=}` opens a literal made of the `L:` lines below
/// L: Subject: Hi              one literal line (joined with CRLF)
/// S+: )                       continues the line the literal interrupted
/// ```
///
/// A command that does not match the script is recorded in
/// [`ReplayServer::mismatches`] and the connection closes.
#[derive(Clone)]
pub struct ReplayServer {
    inner: Arc<Mutex<ReplayState>>,
}

#[derive(Debug, Clone)]
enum ReplayStep {
    Expect(String),
    Send(Vec<u8>),
}

#[derive(Default)]
struct ReplayState {
    connections: VecDeque<VecDeque<ReplayStep>>,
    mismatches: Vec<String>,
    unused: usize,
}

impl ReplayServer {
    pub fn from_transcript(transcript: &str) -> Self {
        let mut connections: VecDeque<VecDeque<ReplayStep>> = VecDeque::new();
        let mut current: VecDeque<ReplayStep> = VecDeque::new();
        let mut literal: Option<(Vec<u8>, Vec<String>)> = None;
        let flush_literal = |current: &mut VecDeque<ReplayStep>, literal: &mut Option<(Vec<u8>, Vec<String>)>| {
            if let Some((mut head, lines)) = literal.take() {
                let body = lines.join("\r\n");
                head.extend(format!("{{{}}}\r\n", body.len()).into_bytes());
                head.extend(body.into_bytes());
                current.push_back(ReplayStep::Send(head));
            }
        };
        for line in transcript.lines() {
            if let Some(rest) = line.strip_prefix("L:") {
                if let Some((_, lines)) = literal.as_mut() {
                    lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                }
                continue;
            }
            flush_literal(&mut current, &mut literal);
            if line.starts_with("=== connection") {
                if !current.is_empty() {
                    connections.push_back(std::mem::take(&mut current));
                }
            } else if let Some(rest) = line.strip_prefix("C: ") {
                current.push_back(ReplayStep::Expect(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("S+:") {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                current.push_back(ReplayStep::Send(format!("{rest}\r\n").into_bytes()));
            } else if let Some(rest) = line.strip_prefix("S: ") {
                match rest.strip_suffix("{=}") {
                    Some(head) => literal = Some((head.as_bytes().to_vec(), Vec::new())),
                    None => current.push_back(ReplayStep::Send(format!("{rest}\r\n").into_bytes())),
                }
            }
        }
        flush_literal(&mut current, &mut literal);
        if !current.is_empty() {
            connections.push_back(current);
        }
        Self {
            inner: Arc::new(Mutex::new(ReplayState {
                connections,
                ..ReplayState::default()
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ReplayState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Commands that did not match the script, with what was expected.
    pub fn mismatches(&self) -> Vec<String> {
        self.lock().mismatches.clone()
    }

    /// Scripted steps never reached, over all connections. Non-zero means the
    /// client stopped earlier than the recording.
    pub fn unused_steps(&self) -> usize {
        let state = self.lock();
        state.unused + state.connections.iter().map(VecDeque::len).sum::<usize>()
    }

    pub fn transport(&self) -> Arc<dyn MailTransport> {
        Arc::new(self.clone())
    }
}

#[async_trait]
impl MailTransport for ReplayServer {
    fn open_imap(&self, _creds: &ImapCredentials) -> std::result::Result<Box<dyn ImapStream>, imap::Error> {
        let script = self.lock().connections.pop_front().ok_or_else(|| {
            imap::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "the transcript has no more connections",
            ))
        })?;
        let mut connection = ReplayConnection {
            server: self.clone(),
            script,
            input: Vec::new(),
            output: VecDeque::new(),
            tag: String::new(),
            closed: false,
        };
        connection.send_until_expect();
        Ok(Box::new(connection))
    }

    async fn send_smtp(&self, _creds: &ImapCredentials, _email: lettre::Message) -> Result<()> {
        Err(AppError::SyncError("ReplayServer does not speak SMTP".to_string()))
    }

    async fn test_smtp(&self, _creds: &ImapCredentials) -> Result<()> {
        Ok(())
    }
}

struct ReplayConnection {
    server: ReplayServer,
    script: VecDeque<ReplayStep>,
    input: Vec<u8>,
    output: VecDeque<u8>,
    tag: String,
    closed: bool,
}

impl Drop for ReplayConnection {
    fn drop(&mut self) {
        self.server.lock().unused += self.script.len();
    }
}

impl ReplayConnection {
    /// Send scripted server output up to the next expected command.
    fn send_until_expect(&mut self) {
        while let Some(ReplayStep::Send(bytes)) = self.script.front() {
            let text = String::from_utf8_lossy(bytes).replace("{tag}", &self.tag);
            self.output.extend(text.into_bytes());
            self.script.pop_front();
        }
        if self.script.is_empty() {
            // The recording ends here: the server hung up.
            self.closed = true;
        }
    }
}

impl Read for ReplayConnection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.output.len());
        for (slot, byte) in buf.iter_mut().zip(self.output.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
}

impl Write for ReplayConnection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.closed {
            // The recording hung up here, as the real server did.
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the recorded server closed the connection",
            ));
        }
        self.input.extend_from_slice(buf);
        while let Some(end) = find_crlf(&self.input) {
            let line: Vec<u8> = self.input.drain(..end + 2).take(end).collect();
            let line = String::from_utf8_lossy(&line).into_owned();
            let (tag, command) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match self.script.pop_front() {
                Some(ReplayStep::Expect(pattern)) if command_matches(&pattern, command) => {
                    self.tag = tag.to_string();
                    self.send_until_expect();
                }
                other => {
                    let expected = match other {
                        Some(ReplayStep::Expect(p)) => p,
                        _ => "<end of transcript>".to_string(),
                    };
                    self.server
                        .lock()
                        .mismatches
                        .push(format!("got `{}`, expected `{expected}`", redact_login(command)));
                    self.script.clear();
                    self.closed = true;
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn command_matches(pattern: &str, command: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => command.starts_with(prefix),
        None => command == pattern,
    }
}
