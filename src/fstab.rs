//! `/etc/fstab` edits that leave every untouched line byte-for-byte.

use std::fmt::Write as _;

const NETWORK_TYPES: &[&str] = &[
    "cifs", "smb", "nfs", "nfs4", "fuse.sshfs", "sshfs", "davfs", "fuse.rclone",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub options: String,
    pub dump: String,
    pub pass: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub id: String,
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub options: String,
}

#[derive(Clone, Debug)]
pub struct ShareDraft {
    pub id: String,
    pub protocol: String,
    pub source: String,
    pub mount_point: String,
    pub guest: bool,
    pub username: String,
    pub password: String,
    pub credentials_file: String,
    pub identity_file: String,
    pub extra: String,
    pub netdev: bool,
    pub nofail: bool,
    pub automount: bool,
    pub idle_timeout: String,
    pub vers: String,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredWrite {
    pub path: String,
    pub contents: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharePlan {
    pub id: String,
    pub source: String,
    pub target: String,
    pub entry_line: String,
    pub credentials: Option<CredWrite>,
}

#[derive(Clone, Debug)]
pub enum Change {
    Upsert {
        id: String,
        source: String,
        target: String,
        line: String,
    },
    Delete {
        id: String,
        source: String,
        target: String,
    },
}

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            ' ' => out.push_str("\\040"),
            '\t' => out.push_str("\\011"),
            '\n' => out.push_str("\\012"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

pub fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'\\' {
                out.push('\\');
                i += 2;
                continue;
            }
            if i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(|b| b.is_ascii_digit()) {
                let oct = &value[i + 1..i + 4];
                if let Ok(code) = u8::from_str_radix(oct, 8) {
                    out.push(code as char);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

pub fn parse_entry(line: &str) -> Option<Fields> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let parts = split_fields(trimmed);
    if parts.len() < 4 {
        return None;
    }
    Some(Fields {
        source: unescape(&parts[0]),
        target: unescape(&parts[1]),
        fstype: parts[2].clone(),
        options: unescape(&parts[3]),
        dump: parts.get(4).cloned().unwrap_or_else(|| "0".into()),
        pass: parts.get(5).cloned().unwrap_or_else(|| "0".into()),
    })
}

fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            current.push('\\');
            if let Some(next) = chars.next() {
                current.push(next);
                // octal escapes are three digits; keep them inside the field
                if next.is_ascii_digit() {
                    for _ in 0..2 {
                        if chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                            current.push(chars.next().unwrap());
                        }
                    }
                }
            }
            continue;
        }
        if c.is_whitespace() {
            if !current.is_empty() {
                fields.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(c);
    }
    if !current.is_empty() {
        fields.push(current);
    }
    fields
}

pub fn is_network(fields: &Fields) -> bool {
    let kind = fields.fstype.to_ascii_lowercase();
    NETWORK_TYPES.iter().any(|t| *t == kind)
        || fields.options.split(',').any(|opt| {
            let key = opt.split('=').next().unwrap_or("").trim();
            key == "_netdev" || key == "x-systemd.automount"
        })
}

pub fn network_shares(text: &str) -> Vec<Share> {
    let lines = lines_of(text);
    let mut shares = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(fields) = parse_entry(line) else {
            continue;
        };
        if !is_network(&fields) {
            continue;
        }
        let id = previous_id(&lines, index).unwrap_or_else(|| slug(&fields.target));
        shares.push(Share {
            id,
            source: fields.source,
            target: fields.target,
            fstype: fields.fstype,
            options: fields.options,
        });
    }
    shares
}

fn previous_id(lines: &[String], index: usize) -> Option<String> {
    if index == 0 {
        return None;
    }
    let prev = lines[index - 1].trim();
    prev.strip_prefix("# disktui:")
        .map(|rest| rest.trim().to_string())
        .filter(|id| !id.is_empty())
}

pub fn slug(target: &str) -> String {
    let mut out = String::new();
    for c in target.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "share".into()
    } else {
        out
    }
}

/// Apply one insert, replace, or delete. Every line that is not the edited
/// entry is copied through unchanged.
pub fn transform(original: &str, change: &Change) -> String {
    let newline_at_eof = original.is_empty() || original.ends_with('\n');
    let mut lines = lines_of(original);
    match change {
        Change::Delete { id, source, target } => {
            if let Some(idx) = find_entry(&lines, id, source, target) {
                let start = comment_start(&lines, idx);
                lines.drain(start..=idx);
            }
        }
        Change::Upsert {
            id,
            source,
            target,
            line,
        } => {
            let comment = format!("# disktui: {id}");
            if let Some(idx) = find_entry(&lines, id, source, target) {
                let start = comment_start(&lines, idx);
                lines.splice(start..=idx, [comment, line.clone()]);
            } else {
                if lines.last().is_some_and(|l| l.is_empty()) {
                    lines.pop();
                }
                if lines.last().is_some_and(|l| !l.is_empty()) {
                    // keep a blank line between the existing file and our entry
                }
                lines.push(comment);
                lines.push(line.clone());
            }
        }
    }
    let mut out = lines.join("\n");
    if newline_at_eof || matches!(change, Change::Upsert { .. }) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if original.is_empty() && matches!(change, Change::Delete { .. }) {
        return String::new();
    }
    out
}

fn lines_of(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    trimmed.split('\n').map(|s| s.to_string()).collect()
}

fn comment_start(lines: &[String], idx: usize) -> usize {
    if idx > 0 && is_disktui_comment(&lines[idx - 1]) {
        idx - 1
    } else {
        idx
    }
}

fn is_disktui_comment(line: &str) -> bool {
    line.trim().starts_with("# disktui:")
}

fn find_entry(lines: &[String], id: &str, source: &str, target: &str) -> Option<usize> {
    let marker = format!("# disktui: {id}");
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == marker {
            if let Some(j) = ((i + 1)..lines.len()).find(|j| parse_entry(&lines[*j]).is_some()) {
                return Some(j);
            }
        }
    }
    lines.iter().position(|line| match parse_entry(line) {
        Some(fields) => fields.source == source && fields.target == target && is_network(&fields),
        None => false,
    })
}

pub fn plan_share(draft: &ShareDraft) -> Result<SharePlan, String> {
    let target = draft.mount_point.trim();
    if !target.starts_with('/') {
        return Err("Mount point must be an absolute path.".into());
    }
    if target.contains('\n') || draft.source.contains('\n') || draft.extra.contains('\n') {
        return Err("Fields cannot contain a newline.".into());
    }
    let source = draft.source.trim();
    if source.is_empty() {
        return Err("Source is empty.".into());
    }
    let protocol = draft.protocol.trim();
    if protocol.is_empty() {
        return Err("Filesystem type is empty.".into());
    }
    let id = {
        let raw = draft.id.trim();
        let id = if raw.is_empty() { slug(target) } else { slug(raw) };
        if id.is_empty() {
            return Err("Share id is empty.".into());
        }
        id
    };

    let mut credentials = None;
    let cred_path = if protocol == "cifs" && !draft.guest {
        if !draft.credentials_file.trim().is_empty() {
            draft.credentials_file.trim().to_string()
        } else if !draft.username.trim().is_empty() {
            let path = format!("/etc/samba/credentials-{id}");
            if draft.password.contains('\n') || draft.username.contains('\n') {
                return Err("Username and password cannot contain a newline.".into());
            }
            credentials = Some(CredWrite {
                path: path.clone(),
                contents: format!(
                    "username={}\npassword={}\n",
                    draft.username.trim(),
                    draft.password
                ),
            });
            path
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let options = build_options(draft, &cred_path);
    let line = format!(
        "{source} {target} {protocol} {options} 0 0",
        source = escape(source),
        target = escape(target),
        protocol = protocol,
        options = escape(&options),
    );
    if line.contains(draft.password.as_str()) && !draft.password.is_empty() && protocol == "cifs" {
        return Err("Refusing to write the password into fstab. Use a credentials file.".into());
    }
    Ok(SharePlan {
        id,
        source: source.to_string(),
        target: target.to_string(),
        entry_line: line,
        credentials,
    })
}

fn build_options(draft: &ShareDraft, cred_path: &str) -> String {
    let managed = [
        "_netdev",
        "nofail",
        "x-systemd.automount",
        "x-systemd.idle-timeout",
        "guest",
        "credentials",
        "username",
        "password",
        "uid",
        "gid",
        "file_mode",
        "dir_mode",
        "iocharset",
        "vers",
        "IdentityFile",
        "identityfile",
        "reconnect",
        "ServerAliveInterval",
    ];
    let mut parts: Vec<String> = Vec::new();
    for opt in draft.extra.split(',') {
        let opt = opt.trim();
        if opt.is_empty() {
            continue;
        }
        let key = opt.split('=').next().unwrap_or("").trim();
        if managed.iter().any(|m| *m == key) {
            continue;
        }
        parts.push(opt.to_string());
    }
    let cifs = draft.protocol == "cifs";
    let ssh = draft.protocol == "fuse.sshfs" || draft.protocol == "sshfs";
    if cifs {
        if draft.guest {
            parts.push("guest".into());
        } else if !cred_path.is_empty() {
            parts.push(format!("credentials={cred_path}"));
        }
        parts.push(format!("uid={}", draft.uid));
        parts.push(format!("gid={}", draft.gid));
        parts.push("file_mode=0644".into());
        parts.push("dir_mode=0755".into());
        parts.push("iocharset=utf8".into());
        if !draft.vers.trim().is_empty() && draft.vers != "default" {
            parts.push(format!("vers={}", draft.vers.trim()));
        }
    }
    if ssh {
        if !draft.identity_file.trim().is_empty() {
            parts.push(format!("IdentityFile={}", draft.identity_file.trim()));
        }
        parts.push("reconnect".into());
        parts.push("ServerAliveInterval=15".into());
    }
    if draft.netdev {
        parts.push("_netdev".into());
    }
    if draft.nofail {
        parts.push("nofail".into());
    }
    if draft.automount {
        parts.push("x-systemd.automount".into());
        if let Ok(seconds) = draft.idle_timeout.trim().parse::<u64>() {
            if seconds > 0 {
                parts.push(format!("x-systemd.idle-timeout={seconds}"));
            }
        }
    }
    if parts.is_empty() {
        "defaults".into()
    } else {
        parts.join(",")
    }
}

pub fn helper_for(fstype: &str) -> Option<&'static str> {
    match fstype {
        "cifs" | "smb" => Some("mount.cifs"),
        "nfs" | "nfs4" => Some("mount.nfs"),
        "fuse.sshfs" | "sshfs" => Some("sshfs"),
        "davfs" => Some("mount.davfs"),
        _ => None,
    }
}

pub fn missing_helper(fstype: &str) -> Option<String> {
    let helper = helper_for(fstype)?;
    let candidates = [
        format!("/usr/bin/{helper}"),
        format!("/usr/sbin/{helper}"),
        format!("/bin/{helper}"),
        format!("/sbin/{helper}"),
    ];
    if candidates.iter().any(|p| std::path::Path::new(p).exists()) {
        None
    } else {
        let package = match fstype {
            "cifs" | "smb" => "cifs-utils",
            "nfs" | "nfs4" => "nfs-utils",
            "fuse.sshfs" | "sshfs" => "sshfs",
            "davfs" => "davfs2",
            _ => helper,
        };
        Some(format!("{helper} is not installed (package {package})"))
    }
}

/// Shell-quote a single argument. Used when the sudo script embeds a path.
pub fn sh_quote(value: &str) -> String {
    let mut out = String::from("'");
    for c in value.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

pub fn render_script(backup: &str, plan_contents: &str, creds: Option<&CredWrite>, mount: Option<&str>) -> String {
    let mut script = String::from("set -eu\n");
    let _ = writeln!(script, "cp -a /etc/fstab {}", sh_quote(backup));
    if let Some(cred) = creds {
        let dir = std::path::Path::new(&cred.path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/etc/samba".into());
        let _ = writeln!(script, "install -d -m 755 {}", sh_quote(&dir));
        let _ = writeln!(script, "cat > {} << 'DISKTUI_CRED'", sh_quote(&cred.path));
        script.push_str(&cred.contents);
        if !cred.contents.ends_with('\n') {
            script.push('\n');
        }
        script.push_str("DISKTUI_CRED\n");
        let _ = writeln!(script, "chmod 600 {}", sh_quote(&cred.path));
    }
    script.push_str("cat > /etc/fstab << 'DISKTUI_FSTAB'\n");
    script.push_str(plan_contents);
    if !plan_contents.ends_with('\n') {
        script.push('\n');
    }
    script.push_str("DISKTUI_FSTAB\n");
    script.push_str("chmod 644 /etc/fstab\n");
    if let Some(target) = mount {
        let _ = writeln!(script, "mount -- {}", sh_quote(target));
    }
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Static information about the filesystems.\n\
UUID=9574fe74-248e-45d8-bdbf-269b1ddcfd9f\t/\tbtrfs\trw,relatime,subvol=/@\t0\t0\n\
UUID=0356-B9FA  /boot  vfat  rw,relatime  0  2\n\
/swap/swapfile none swap defaults 0 0\n";

    fn draft() -> ShareDraft {
        ShareDraft {
            id: String::new(),
            protocol: "cifs".into(),
            source: "//nas/media".into(),
            mount_point: "/mnt/media".into(),
            guest: false,
            username: "ken".into(),
            password: "s3cret".into(),
            credentials_file: String::new(),
            identity_file: String::new(),
            extra: String::new(),
            netdev: true,
            nofail: true,
            automount: false,
            idle_timeout: String::new(),
            vers: "3.1.1".into(),
            uid: 1000,
            gid: 1000,
        }
    }

    #[test]
    fn upsert_preserves_unrelated_lines() {
        let plan = plan_share(&draft()).unwrap();
        assert!(!plan.entry_line.contains("s3cret"));
        assert!(plan.entry_line.contains("credentials=/etc/samba/credentials-mnt-media"));
        let next = transform(
            SAMPLE,
            &Change::Upsert {
                id: plan.id.clone(),
                source: plan.source.clone(),
                target: plan.target.clone(),
                line: plan.entry_line.clone(),
            },
        );
        assert!(next.contains("UUID=9574fe74-248e-45d8-bdbf-269b1ddcfd9f\t/\tbtrfs\trw,relatime,subvol=/@\t0\t0\n"));
        assert!(next.contains("UUID=0356-B9FA  /boot  vfat  rw,relatime  0  2\n"));
        assert!(next.contains("# disktui: mnt-media\n"));
        assert!(next.contains(&plan.entry_line));
        let again = transform(
            &next,
            &Change::Upsert {
                id: "mnt-media".into(),
                source: "//nas/media".into(),
                target: "/mnt/media".into(),
                line: "//nas/media /mnt/photos cifs credentials=/etc/samba/credentials-mnt-media,_netdev 0 0".into(),
            },
        );
        assert_eq!(again.matches("# disktui:").count(), 1);
        assert!(again.contains("/mnt/photos"));
        assert!(!again.contains("/mnt/media cifs"));
        assert!(again.contains("subvol=/@"));
    }

    #[test]
    fn delete_removes_only_the_marked_entry() {
        let plan = plan_share(&draft()).unwrap();
        let with = transform(
            SAMPLE,
            &Change::Upsert {
                id: plan.id.clone(),
                source: plan.source,
                target: plan.target.clone(),
                line: plan.entry_line,
            },
        );
        let cleared = transform(
            &with,
            &Change::Delete {
                id: "mnt-media".into(),
                source: "//nas/media".into(),
                target: "/mnt/media".into(),
            },
        );
        assert_eq!(cleared, SAMPLE);
    }

    #[test]
    fn escapes_spaces_in_the_mount_point() {
        let mut d = draft();
        d.mount_point = "/mnt/my media".into();
        d.guest = true;
        d.password.clear();
        let plan = plan_share(&d).unwrap();
        assert!(plan.entry_line.contains("/mnt/my\\040media"));
        let fields = parse_entry(&plan.entry_line).unwrap();
        assert_eq!(fields.target, "/mnt/my media");
        assert!(fields.options.contains("guest"));
        assert!(!fields.options.contains("credentials="));
    }

    #[test]
    fn nfs_and_existing_manual_entry() {
        let mut d = draft();
        d.protocol = "nfs".into();
        d.source = "nas:/export".into();
        d.mount_point = "/mnt/export".into();
        let plan = plan_share(&d).unwrap();
        assert!(plan.credentials.is_none());
        assert!(plan.entry_line.contains("nfs"));
        let manual = format!("{SAMPLE}nas:/export /mnt/export nfs _netdev 0 0\n");
        let next = transform(
            &manual,
            &Change::Upsert {
                id: plan.id,
                source: plan.source,
                target: plan.target,
                line: plan.entry_line,
            },
        );
        assert_eq!(next.matches("nas:/export").count(), 1);
        assert!(next.contains("# disktui:"));
        let shares = network_shares(&next);
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0].fstype, "nfs");
    }

    #[test]
    fn password_never_lands_in_the_fstab_line() {
        let plan = plan_share(&draft()).unwrap();
        let creds = plan.credentials.unwrap();
        assert!(creds.contents.contains("password=s3cret"));
        assert!(!plan.entry_line.contains("s3cret"));
    }
}
