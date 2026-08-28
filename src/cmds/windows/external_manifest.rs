//! Offline manifest of recognized external Desktop Windows commands.
//!
//! Snapshot provenance: the Microsoft Learn [Windows Commands] catalog applies
//! to Windows 10 and Windows 11 and was last updated 2026-02-24. This manifest
//! was reviewed on 2026-08-29 and is deliberately checked in: neither runtime
//! nor builds fetch the catalog. It records recognition only; every entry is
//! raw until a later, command-specific adapter earns an incremental status.
//!
//! [Windows Commands]: https://learn.microsoft.com/windows-server/administration/windows-commands/windows-commands

/// Desktop support recorded for an external command in this manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandAvailability {
    DesktopWindows10And11,
}

/// The intentionally conservative adapter policy for external commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStrategy {
    IdentityRaw,
}

/// Rollout state for an external command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStatus {
    RecognizedRaw,
}

/// Checked-in metadata for one external `cmd.exe` command and its aliases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalCommand {
    pub name: &'static str,
    pub aliases: Vec<&'static str>,
    pub availability: Option<CommandAvailability>,
    pub strategy: Option<ExternalStrategy>,
    pub status: Option<ExternalStatus>,
}

/// Return the first stable external-command manifest.
///
/// The first increment recognizes these names but does not route or filter
/// them. `rtk cmd` therefore preserves native `cmd.exe` execution and output.
pub fn external_commands() -> Vec<ExternalCommand> {
    [
        "append",
        "arp",
        "attrib",
        "bcdboot",
        "bcdedit",
        "bitsadmin",
        "bootcfg",
        "cacls",
        "certreq",
        "certutil",
        "change",
        "chkdsk",
        "chkntfs",
        "choice",
        "cipher",
        "cleanmgr",
        "clip",
        "comp",
        "compact",
        "convert",
        "defrag",
        "diskpart",
        "diskperf",
        "doskey",
        "driverquery",
        "eventcreate",
        "eventquery",
        "eventtriggers",
        "expand",
        "extract",
        "fc",
        "find",
        "findstr",
        "fltmc",
        "forfiles",
        "format",
        "fsutil",
        "ftp",
        "getmac",
        "gpresult",
        "gpupdate",
        "hostname",
        "icacls",
        "ipconfig",
        "label",
        "lodctr",
        "logman",
        "manage-bde",
        "mdsched",
        "mode",
        "more",
        "mountvol",
        "msinfo32",
        "nbtstat",
        "net",
        "netsh",
        "netstat",
        "nslookup",
        "openfiles",
        "pathping",
        "perfmon",
        "ping",
        "pnputil",
        "powercfg",
        "print",
        "query",
        "quser",
        "qwinsta",
        "rasdial",
        "recover",
        "reg",
        "regini",
        "regsvr32",
        "relog",
        "replace",
        "robocopy",
        "route",
        "runas",
        "sc",
        "schtasks",
        "setx",
        "sfc",
        "shadow",
        "shutdown",
        "sort",
        "subst",
        "systeminfo",
        "takeown",
        "taskkill",
        "tasklist",
        "timeout",
        "tracerpt",
        "tracert",
        "tree",
        "tscon",
        "tsdiscon",
        "tskill",
        "typeperf",
        "tzutil",
        "verifier",
        "vssadmin",
        "wecutil",
        "wevtutil",
        "where",
        "whoami",
        "winrs",
        "winrm",
        "winsat",
        "wmic",
        "wsreset",
        "wusa",
        "xcopy",
    ]
    .into_iter()
    .map(raw)
    .collect()
}

fn raw(name: &'static str) -> ExternalCommand {
    ExternalCommand {
        name,
        aliases: vec![],
        availability: Some(CommandAvailability::DesktopWindows10And11),
        strategy: Some(ExternalStrategy::IdentityRaw),
        status: Some(ExternalStatus::RecognizedRaw),
    }
}
