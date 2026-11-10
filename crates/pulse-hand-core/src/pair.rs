//! The pairing side of the `pulse-hands` protocol (ADR-070 step 1c): where
//! a paired machine keeps `pulse-hands` and `pulse`, the sshd drop-in and
//! the forced-command key line that bind the hands' user to its verbs, the
//! `step=NAME result=WORD` lines a pairing script prints and the inspection
//! it answers, and the undo and commit scripts. The brain's `pair` composes
//! the pairing from these; a hand reads the same lines it writes.

use base64::Engine;
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub const PAIR_DROPIN_PATH: &str = "/etc/ssh/sshd_config.d/60-pulse-hands.conf";
pub const PAIR_HANDS_PATH: &str = "/usr/local/bin/pulse-hands";
pub const PAIR_PULSE_PATH: &str = "/usr/local/bin/pulse";
pub static STEP_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^step=([A-Za-z0-9_-]+)[ \t]+result=([A-Za-z0-9_-]+)").unwrap()
});

pub fn valid_name(name: String) -> bool {
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[a-z][a-z0-9-]*$").unwrap());
    RE.is_match(&name)
}

pub fn pair_dropin_text() -> String {
    [
        "# The hands' user gets exactly what its authorized_keys line allows and nothing else.",
        "# The hub runs every session to these hands over one shared connection, so the",
        "# sessions it has open at once (a job, its steps mirror, a probe) share this bound.",
        "Match User pulse",
        "    PasswordAuthentication no",
        "    AllowAgentForwarding no",
        "    AllowTcpForwarding no",
        "    X11Forwarding no",
        "    PermitTTY no",
        "    PermitTunnel no",
        "    MaxSessions 64",
        "",
    ]
    .join("\n")
}

pub fn pair_authorized_key_line(workdir: String, pubkey: String) -> String {
    format!(
        "command=\"PULSE_HANDS_WORKDIR={} {} hands\",restrict {}",
        workdir,
        PAIR_HANDS_PATH,
        pubkey.trim()
    )
}

pub fn pair_parse_inspection(text: String) -> (BTreeMap<String, String>, Vec<String>) {
    let mut fields = BTreeMap::new();
    let mut hostkeys = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("hostkey ") {
            hostkeys.push(rest.trim().to_string());
        } else if let Some((k, v)) = line.split_once('=') {
            fields.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    (fields, hostkeys)
}

pub fn pair_tail(text: String) -> String {
    crate::transport::tail(&text.split_whitespace().collect::<Vec<_>>().join(" "), 500)
}

pub fn wrap_base64(data: Vec<u8>) -> String {
    let enc = base64::engine::general_purpose::STANDARD.encode(data);
    enc.as_bytes()
        .chunks(76)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn pair_undo_script() -> String {
    format!(
        "#!/bin/sh\nset -eu\nDROPIN={}\nif [ -f \"$DROPIN.prev\" ]; then\n  mv -f \"$DROPIN.prev\" \"$DROPIN\"\nelse\n  rm -f \"$DROPIN\"\nfi\nsystemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || systemctl restart ssh 2>/dev/null || systemctl restart sshd\nprintf 'step=dropin result=restored-or-removed\\n'\n",
        crate::util::shell_quote(PAIR_DROPIN_PATH)
    )
}

pub fn pair_commit_script() -> String {
    format!(
        "#!/bin/sh\nset -eu\nrm -f {}.prev\nprintf 'step=dropin-prev result=removed\\n'\n",
        crate::util::shell_quote(PAIR_DROPIN_PATH)
    )
}

pub fn pair_step_results(text: String) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if let Some(c) = STEP_RE.captures(line.trim()) {
            out.insert(c[1].to_string(), c[2].to_string());
        }
    }
    out
}

pub fn go_arch_for(uname: String) -> String {
    match uname.as_str() {
        "x86_64" | "amd64" => "amd64".into(),
        "aarch64" | "arm64" => "arm64".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_line_forces_the_hands_verbs() {
        let line = pair_authorized_key_line("/srv/pulse".into(), " ssh-ed25519 AAAA hub\n".into());
        assert_eq!(
            line,
            "command=\"PULSE_HANDS_WORKDIR=/srv/pulse /usr/local/bin/pulse-hands hands\",restrict ssh-ed25519 AAAA hub"
        );
        assert!(pair_dropin_text().contains("Match User pulse"));
    }

    #[test]
    fn step_lines_and_inspection_fields_are_read_back() {
        let steps = pair_step_results(
            "noise\nstep=user result=created\n  step=dropin\tresult=done\n".into(),
        );
        assert_eq!(steps.get("user").map(String::as_str), Some("created"));
        assert_eq!(steps.get("dropin").map(String::as_str), Some("done"));
        let (fields, keys) =
            pair_parse_inspection("arch=x86_64\nhostkey ssh-ed25519 AAAA\n".into());
        assert_eq!(fields.get("arch").map(String::as_str), Some("x86_64"));
        assert_eq!(keys, vec!["ssh-ed25519 AAAA".to_string()]);
        assert_eq!(wrap_base64(vec![0u8; 120]).lines().count(), 3);
        assert!(valid_name("hands-1".into()) && !valid_name("1hands".into()));
    }
}
