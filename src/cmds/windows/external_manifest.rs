//! Checked-in, offline Desktop CMD external-command snapshot.
//!
//! Source: Microsoft Learn's Windows Commands A-Z catalog, last updated
//! 2026-02-24. The catalog landing page applies to Windows 10 and Windows 11.
//! This is a reviewed snapshot, not a runtime probe: builds and execution never
//! fetch it. The coverage table records why each audited top-level A-Z name is
//! routed here, owned by the CMD built-in catalog, server-only, or a subcommand.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    MicrosoftWindowsCommandsAz20260224,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionStatus {
    Supported,
    Deprecated,
    Unsupported,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presence {
    Inbox,
    OptionalFeature,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesktopSupport {
    pub win10: VersionStatus,
    pub win11: VersionStatus,
    pub presence: Presence,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalRoute {
    NativeExecutable,
}

/// Bit-set command behavior metadata used to keep raw execution conservative.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandModes(u8);
impl CommandModes {
    pub const QUERY: Self = Self(1 << 0);
    pub const MUTATION: Self = Self(1 << 1);
    pub const INTERACTIVE: Self = Self(1 << 2);
    pub const STRUCTURED: Self = Self(1 << 3);
    pub const MACHINE: Self = Self(1 << 4);
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    #[cfg(test)]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStrategy {
    IdentityRaw,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStatus {
    RecognizedRaw,
}

/// Fully typed metadata for a recognized external command family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub route: ExternalRoute,
    pub desktop: DesktopSupport,
    pub modes: CommandModes,
    pub strategy: ExternalStrategy,
    pub identity_reason: &'static str,
    pub status: ExternalStatus,
    pub provenance: Provenance,
}

/// Audited disposition for each included top-level A-Z snapshot name.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogDisposition {
    DesktopExternal,
    CmdBuiltin,
    UnsupportedOnDesktop,
    OptionalDesktopFeature,
    ServerOnly,
    SubcommandOnly,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogCoverage {
    pub name: &'static str,
    pub disposition: CatalogDisposition,
    pub provenance: Provenance,
}

const SOURCE: Provenance = Provenance::MicrosoftWindowsCommandsAz20260224;
const DESKTOP: DesktopSupport = DesktopSupport {
    win10: VersionStatus::Supported,
    win11: VersionStatus::Supported,
    presence: Presence::Inbox,
};
const OPTIONAL: DesktopSupport = DesktopSupport {
    win10: VersionStatus::Supported,
    win11: VersionStatus::Deprecated,
    presence: Presence::OptionalFeature,
};
const RAW_REASON: &str = "no external adapter is released in the stable CMD increment";

macro_rules! external {
    ($name:literal, $modes:expr) => { ExternalCommand { name: $name, aliases: &[], route: ExternalRoute::NativeExecutable, desktop: DESKTOP, modes: $modes, strategy: ExternalStrategy::IdentityRaw, identity_reason: RAW_REASON, status: ExternalStatus::RecognizedRaw, provenance: SOURCE } };
    ($name:literal, [$($alias:literal),+], $modes:expr) => { ExternalCommand { name: $name, aliases: &[$($alias),+], route: ExternalRoute::NativeExecutable, desktop: DESKTOP, modes: $modes, strategy: ExternalStrategy::IdentityRaw, identity_reason: RAW_REASON, status: ExternalStatus::RecognizedRaw, provenance: SOURCE } };
    (optional $name:literal, $modes:expr) => { ExternalCommand { name: $name, aliases: &[], route: ExternalRoute::NativeExecutable, desktop: OPTIONAL, modes: $modes, strategy: ExternalStrategy::IdentityRaw, identity_reason: RAW_REASON, status: ExternalStatus::RecognizedRaw, provenance: SOURCE } };
}
#[cfg(test)]
macro_rules! coverage {
    ($name:literal, $disposition:ident) => {
        CatalogCoverage {
            name: $name,
            disposition: CatalogDisposition::$disposition,
            provenance: SOURCE,
        }
    };
}

const Q: CommandModes = CommandModes::QUERY;
const QM: CommandModes = Q.union(CommandModes::MACHINE);
const QS: CommandModes = Q.union(CommandModes::STRUCTURED);
const M: CommandModes = CommandModes::MUTATION;
const MI: CommandModes = M.union(CommandModes::INTERACTIVE);
const I: CommandModes = CommandModes::INTERACTIVE;

/// Immutable recognized Desktop command families. All execution remains raw.
pub static EXTERNAL_COMMANDS: &[ExternalCommand] = &[
    external!("arp", QM),
    external!("assign", M),
    external!("attrib", M),
    external!("auditpol", QS),
    external!("autochk", Q),
    external!("bcdboot", M),
    external!("bcdedit", QS),
    external!("bdehdcfg", M),
    external!("bitsadmin", QS),
    external!("cacls", M),
    external!("certreq", MI),
    external!("certutil", QS),
    external!("change", ["chglogon", "chgport", "chgusr"], M),
    external!("chkdsk", MI),
    external!("chkntfs", QM),
    external!("choice", I),
    external!("cipher", M),
    external!("cleanmgr", I),
    external!("clip", M),
    external!("cmdkey", M),
    external!("cmstp", MI),
    external!("comp", QM),
    external!("compact", M),
    external!("convert", M),
    external!("cscript", I),
    external!("defrag", MI),
    external!("diantz", M),
    external!("diskcomp", QM),
    external!("diskcopy", MI),
    external!("diskpart", MI),
    external!("diskperf", M),
    external!("diskshadow", MI),
    external!("dispdiag", QM),
    external!("doskey", M),
    external!("driverquery", QS),
    external!("dtrace", MI),
    external!("eventcreate", M),
    external!("expand", M),
    external!("fc", QM),
    external!("find", QM),
    external!("findstr", QM),
    external!("fondue", M),
    external!("forfiles", M),
    external!("format", MI),
    external!("fsutil", MI),
    external!("ftp", I),
    external!("fveupdate", M),
    external!("getmac", QS),
    external!("gpresult", QS),
    external!("gpupdate", M),
    external!("hostname", QM),
    external!("icacls", M),
    external!("ipconfig", QS),
    external!("klist", QS),
    external!("label", MI),
    external!("lodctr", M),
    external!("logman", MI),
    external!("logoff", M),
    external!("makecab", M),
    external!("manage-bde", MI),
    external!("mode", MI),
    external!("more", I),
    external!("mountvol", M),
    external!("msg", I),
    external!("msiexec", MI),
    external!("msinfo32", I),
    external!("mstsc", I),
    external!("nbtstat", QS),
    external!("net", MI),
    external!("netcfg", M),
    external!("netsh", MI),
    external!("netstat", QS),
    external!("nslookup", I),
    external!("openfiles", QS),
    external!("pathping", I),
    external!("perfmon", I),
    external!("ping", I),
    external!("pktmon", MI),
    external!("pnputil", M),
    external!("powercfg", MI),
    external!("print", M),
    external!("query", ["qappsrv", "qprocess", "quser", "qwinsta"], QS),
    external!("rasdial", MI),
    external!("rdpsign", M),
    external!("recover", M),
    external!("reg", M),
    external!("regini", M),
    external!("regsvr32", MI),
    external!("relog", M),
    external!("replace", M),
    external!("robocopy", M),
    external!("route", M),
    external!("runas", I),
    external!("rundll32", MI),
    external!("sc", MI),
    external!("schtasks", M),
    external!("secedit", M),
    external!("setspn", M),
    external!("setx", M),
    external!("sfc", MI),
    external!("shadow", M),
    external!("shutdown", MI),
    external!("sort", QM),
    external!("subst", M),
    external!("sxstrace", MI),
    external!("systeminfo", QS),
    external!("takeown", M),
    external!("taskkill", M),
    external!("tasklist", QS),
    external!("timeout", I),
    external!("tracerpt", M),
    external!("tracert", I),
    external!("tree", QM),
    external!("tscon", M),
    external!("tsdiscon", M),
    external!("tskill", M),
    external!("typeperf", QS),
    external!("tzutil", M),
    external!("verifier", MI),
    external!("vssadmin", MI),
    external!("waitfor", I),
    external!("wbadmin", MI),
    external!("wecutil", M),
    external!("wevtutil", QS),
    external!("where", QM),
    external!("whoami", QS),
    external!("winrs", I),
    external!("winsat", MI),
    external!(optional "wmic", QS),
    external!("wscript", I),
    external!("xcopy", M),
];

/// Static A-Z fixture: Desktop externals, CMD built-ins, and non-routable names.
#[cfg(test)]
pub static OFFICIAL_TOP_LEVEL_COVERAGE: &[CatalogCoverage] = &[
    coverage!("active", SubcommandOnly),
    coverage!("add", SubcommandOnly),
    coverage!("adprep", ServerOnly),
    coverage!("append", UnsupportedOnDesktop),
    coverage!("arp", DesktopExternal),
    coverage!("assign", DesktopExternal),
    coverage!("assoc", CmdBuiltin),
    coverage!("at", UnsupportedOnDesktop),
    coverage!("atmadm", ServerOnly),
    coverage!("attach-vdisk", SubcommandOnly),
    coverage!("attrib", DesktopExternal),
    coverage!("attributes", SubcommandOnly),
    coverage!("auditpol", DesktopExternal),
    coverage!("autochk", DesktopExternal),
    coverage!("autoconv", SubcommandOnly),
    coverage!("autofmt", SubcommandOnly),
    coverage!("automount", SubcommandOnly),
    coverage!("bcdboot", DesktopExternal),
    coverage!("bcdedit", DesktopExternal),
    coverage!("bdehdcfg", DesktopExternal),
    coverage!("bitsadmin", DesktopExternal),
    coverage!("bootcfg", UnsupportedOnDesktop),
    coverage!("break", CmdBuiltin),
    coverage!("cacls", DesktopExternal),
    coverage!("call", CmdBuiltin),
    coverage!("cd", CmdBuiltin),
    coverage!("certreq", DesktopExternal),
    coverage!("certutil", DesktopExternal),
    coverage!("change", DesktopExternal),
    coverage!("chcp", CmdBuiltin),
    coverage!("chdir", CmdBuiltin),
    coverage!("chglogon", DesktopExternal),
    coverage!("chgport", DesktopExternal),
    coverage!("chgusr", DesktopExternal),
    coverage!("chkdsk", DesktopExternal),
    coverage!("chkntfs", DesktopExternal),
    coverage!("choice", DesktopExternal),
    coverage!("cipher", DesktopExternal),
    coverage!("clean", SubcommandOnly),
    coverage!("cleanmgr", DesktopExternal),
    coverage!("clip", DesktopExternal),
    coverage!("cls", CmdBuiltin),
    coverage!("cmd", CmdBuiltin),
    coverage!("cmdkey", DesktopExternal),
    coverage!("cmstp", DesktopExternal),
    coverage!("color", CmdBuiltin),
    coverage!("comp", DesktopExternal),
    coverage!("compact", DesktopExternal),
    coverage!("convert", DesktopExternal),
    coverage!("copy", CmdBuiltin),
    coverage!("create", SubcommandOnly),
    coverage!("cscript", DesktopExternal),
    coverage!("date", CmdBuiltin),
    coverage!("dcdiag", ServerOnly),
    coverage!("dcgpofix", ServerOnly),
    coverage!("dcpromo", ServerOnly),
    coverage!("defrag", DesktopExternal),
    coverage!("del", CmdBuiltin),
    coverage!("delete", SubcommandOnly),
    coverage!("detach-vdisk", SubcommandOnly),
    coverage!("detail", SubcommandOnly),
    coverage!("dfsdiag", ServerOnly),
    coverage!("dfsrmig", ServerOnly),
    coverage!("diantz", DesktopExternal),
    coverage!("dir", CmdBuiltin),
    coverage!("diskcomp", DesktopExternal),
    coverage!("diskcopy", DesktopExternal),
    coverage!("diskpart", DesktopExternal),
    coverage!("diskperf", DesktopExternal),
    coverage!("diskraid", ServerOnly),
    coverage!("diskshadow", DesktopExternal),
    coverage!("dispdiag", DesktopExternal),
    coverage!("dnscmd", ServerOnly),
    coverage!("doskey", DesktopExternal),
    coverage!("driverquery", DesktopExternal),
    coverage!("dtrace", DesktopExternal),
    coverage!("echo", CmdBuiltin),
    coverage!("edit", UnsupportedOnDesktop),
    coverage!("endlocal", CmdBuiltin),
    coverage!("eventcreate", DesktopExternal),
    coverage!("exec", SubcommandOnly),
    coverage!("exit", CmdBuiltin),
    coverage!("expand", DesktopExternal),
    coverage!("expose", SubcommandOnly),
    coverage!("extend", SubcommandOnly),
    coverage!("extract", UnsupportedOnDesktop),
    coverage!("fc", DesktopExternal),
    coverage!("filesystems", SubcommandOnly),
    coverage!("find", DesktopExternal),
    coverage!("findstr", DesktopExternal),
    coverage!("finger", OptionalDesktopFeature),
    coverage!("flattemp", ServerOnly),
    coverage!("fondue", DesktopExternal),
    coverage!("for", CmdBuiltin),
    coverage!("forfiles", DesktopExternal),
    coverage!("format", DesktopExternal),
    coverage!("fsutil", DesktopExternal),
    coverage!("ftp", DesktopExternal),
    coverage!("ftype", CmdBuiltin),
    coverage!("fveupdate", DesktopExternal),
    coverage!("getmac", DesktopExternal),
    coverage!("goto", CmdBuiltin),
    coverage!("gpresult", DesktopExternal),
    coverage!("gpupdate", DesktopExternal),
    coverage!("help", CmdBuiltin),
    coverage!("hostname", DesktopExternal),
    coverage!("icacls", DesktopExternal),
    coverage!("if", CmdBuiltin),
    coverage!("import", SubcommandOnly),
    coverage!("ipconfig", DesktopExternal),
    coverage!("klist", DesktopExternal),
    coverage!("ksetup", ServerOnly),
    coverage!("label", DesktopExternal),
    coverage!("list", SubcommandOnly),
    coverage!("lodctr", DesktopExternal),
    coverage!("logman", DesktopExternal),
    coverage!("logoff", DesktopExternal),
    coverage!("makecab", DesktopExternal),
    coverage!("manage-bde", DesktopExternal),
    coverage!("md", CmdBuiltin),
    coverage!("merge", SubcommandOnly),
    coverage!("mkdir", CmdBuiltin),
    coverage!("mklink", CmdBuiltin),
    coverage!("mode", DesktopExternal),
    coverage!("more", DesktopExternal),
    coverage!("mount", OptionalDesktopFeature),
    coverage!("mountvol", DesktopExternal),
    coverage!("move", CmdBuiltin),
    coverage!("msg", DesktopExternal),
    coverage!("msiexec", DesktopExternal),
    coverage!("msinfo32", DesktopExternal),
    coverage!("mstsc", DesktopExternal),
    coverage!("nbtstat", DesktopExternal),
    coverage!("net", DesktopExternal),
    coverage!("netcfg", DesktopExternal),
    coverage!("netdom", ServerOnly),
    coverage!("netsh", DesktopExternal),
    coverage!("netstat", DesktopExternal),
    coverage!("nslookup", DesktopExternal),
    coverage!("offline", SubcommandOnly),
    coverage!("online", SubcommandOnly),
    coverage!("openfiles", DesktopExternal),
    coverage!("path", CmdBuiltin),
    coverage!("pathping", DesktopExternal),
    coverage!("pause", CmdBuiltin),
    coverage!("perfmon", DesktopExternal),
    coverage!("ping", DesktopExternal),
    coverage!("pktmon", DesktopExternal),
    coverage!("pnputil", DesktopExternal),
    coverage!("powercfg", DesktopExternal),
    coverage!("popd", CmdBuiltin),
    coverage!("print", DesktopExternal),
    coverage!("prompt", CmdBuiltin),
    coverage!("pushd", CmdBuiltin),
    coverage!("qappsrv", DesktopExternal),
    coverage!("qprocess", DesktopExternal),
    coverage!("query", DesktopExternal),
    coverage!("quser", DesktopExternal),
    coverage!("qwinsta", DesktopExternal),
    coverage!("rasdial", DesktopExternal),
    coverage!("rd", CmdBuiltin),
    coverage!("rdpsign", DesktopExternal),
    coverage!("recover", DesktopExternal),
    coverage!("reg", DesktopExternal),
    coverage!("regini", DesktopExternal),
    coverage!("regsvr32", DesktopExternal),
    coverage!("relog", DesktopExternal),
    coverage!("rem", CmdBuiltin),
    coverage!("ren", CmdBuiltin),
    coverage!("replace", DesktopExternal),
    coverage!("robocopy", DesktopExternal),
    coverage!("route", DesktopExternal),
    coverage!("runas", DesktopExternal),
    coverage!("rundll32", DesktopExternal),
    coverage!("sc", DesktopExternal),
    coverage!("schtasks", DesktopExternal),
    coverage!("secedit", DesktopExternal),
    coverage!("select", SubcommandOnly),
    coverage!("set", CmdBuiltin),
    coverage!("setlocal", CmdBuiltin),
    coverage!("setspn", DesktopExternal),
    coverage!("setx", DesktopExternal),
    coverage!("sfc", DesktopExternal),
    coverage!("shadow", DesktopExternal),
    coverage!("shift", CmdBuiltin),
    coverage!("shutdown", DesktopExternal),
    coverage!("sort", DesktopExternal),
    coverage!("start", CmdBuiltin),
    coverage!("subst", DesktopExternal),
    coverage!("sxstrace", DesktopExternal),
    coverage!("systeminfo", DesktopExternal),
    coverage!("takeown", DesktopExternal),
    coverage!("taskkill", DesktopExternal),
    coverage!("tasklist", DesktopExternal),
    coverage!("telnet", OptionalDesktopFeature),
    coverage!("time", CmdBuiltin),
    coverage!("timeout", DesktopExternal),
    coverage!("title", CmdBuiltin),
    coverage!("tracerpt", DesktopExternal),
    coverage!("tracert", DesktopExternal),
    coverage!("tree", DesktopExternal),
    coverage!("tscon", DesktopExternal),
    coverage!("tsdiscon", DesktopExternal),
    coverage!("tskill", DesktopExternal),
    coverage!("type", CmdBuiltin),
    coverage!("typeperf", DesktopExternal),
    coverage!("tzutil", DesktopExternal),
    coverage!("unexpose", SubcommandOnly),
    coverage!("ver", CmdBuiltin),
    coverage!("verifier", DesktopExternal),
    coverage!("verify", CmdBuiltin),
    coverage!("vol", CmdBuiltin),
    coverage!("vssadmin", DesktopExternal),
    coverage!("waitfor", DesktopExternal),
    coverage!("wbadmin", DesktopExternal),
    coverage!("wecutil", DesktopExternal),
    coverage!("wevtutil", DesktopExternal),
    coverage!("where", DesktopExternal),
    coverage!("whoami", DesktopExternal),
    coverage!("winrs", DesktopExternal),
    coverage!("winsat", DesktopExternal),
    coverage!("wmic", DesktopExternal),
    coverage!("wscript", DesktopExternal),
    coverage!("xcopy", DesktopExternal),
];

/// Static lookup used by CMD orchestration to recognize raw external commands.
pub fn classify_external(name: &str) -> Option<&'static ExternalCommand> {
    EXTERNAL_COMMANDS.iter().find(|entry| {
        entry.name.eq_ignore_ascii_case(name)
            || entry
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    })
}
#[cfg(test)]
pub const fn external_commands() -> &'static [ExternalCommand] {
    EXTERNAL_COMMANDS
}
#[cfg(test)]
pub const fn official_top_level_coverage() -> &'static [CatalogCoverage] {
    OFFICIAL_TOP_LEVEL_COVERAGE
}

#[cfg(test)]
pub fn validate_external_manifest() -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for command in EXTERNAL_COMMANDS {
        for name in std::iter::once(command.name).chain(command.aliases.iter().copied()) {
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(format!("duplicate external command name or alias: {name}"));
            }
            if !OFFICIAL_TOP_LEVEL_COVERAGE.iter().any(|coverage| {
                coverage.name.eq_ignore_ascii_case(name)
                    && coverage.disposition == CatalogDisposition::DesktopExternal
            }) {
                return Err(format!("{name} is absent from the A-Z desktop fixture"));
            }
        }
    }

    let mut coverage_names = std::collections::HashSet::new();
    for coverage in OFFICIAL_TOP_LEVEL_COVERAGE {
        if !coverage_names.insert(coverage.name.to_ascii_lowercase()) {
            return Err(format!("duplicate A-Z fixture name: {}", coverage.name));
        }
        if coverage.disposition == CatalogDisposition::DesktopExternal
            && classify_external(coverage.name).is_none()
        {
            return Err(format!("{} lacks a Desktop external record", coverage.name));
        }
    }
    Ok(())
}
