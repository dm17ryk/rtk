//! Offline Desktop CMD external-command manifest. Its checked-in source fixture
//! is tests/fixtures/windows_cmd/windows_commands_az.tsv; no build or runtime fetches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    MicrosoftWindowsCommandsAz20250729,
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
    SeparateInstall,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Win11Support {
    pub before_24h2: VersionStatus,
    pub from_24h2: VersionStatus,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Win10Support {
    pub before_21h1: VersionStatus,
    pub from_21h1: VersionStatus,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesktopSupport {
    pub win10: Win10Support,
    pub win11: Win11Support,
    pub presence: Presence,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalRoute {
    NativeExecutable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandModes(u8);
#[allow(dead_code)]
impl CommandModes {
    pub const QUERY: Self = Self(1);
    pub const MUTATION: Self = Self(2);
    pub const INTERACTIVE: Self = Self(4);
    pub const STRUCTURED: Self = Self(8);
    pub const MACHINE: Self = Self(16);
    pub const CONSERVATIVE_ANY: Self = Self(31);
    pub const fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
    #[cfg(test)]
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
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
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogDisposition {
    DesktopExternal,
    CmdBuiltin,
    UnsupportedOnDesktop,
    OptionalDesktopFeature,
    SeparateInstall,
    ServerOnly,
    SubcommandOnly,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogCoverage {
    pub source_name: &'static str,
    pub normalized_name: &'static str,
    pub disposition: CatalogDisposition,
    pub provenance: Provenance,
}
#[cfg(test)]
pub const OFFICIAL_SOURCE_RAW_SHA256: &str =
    "b177c3014e3fa42294ed6fd5356d4bfa08e1a58a8b841e383dd5bcdc01837cc7";
#[cfg(test)]
pub const OFFICIAL_SOURCE_ENTRY_COUNT: usize = 339;
#[cfg(test)]
pub const OFFICIAL_SOURCE_FIXTURE_SHA256: &str =
    "12311903092fb16f3c9596394b7d5e1fafa0badfa354c9e704a68898b961d45c";
const SOURCE: Provenance = Provenance::MicrosoftWindowsCommandsAz20250729;
const W11: Win11Support = Win11Support {
    before_24h2: VersionStatus::Supported,
    from_24h2: VersionStatus::Supported,
};
const W10: Win10Support = Win10Support {
    before_21h1: VersionStatus::Supported,
    from_21h1: VersionStatus::Supported,
};
const D: DesktopSupport = DesktopSupport {
    win10: W10,
    win11: W11,
    presence: Presence::Inbox,
};
const O: DesktopSupport = DesktopSupport {
    win10: W10,
    win11: W11,
    presence: Presence::OptionalFeature,
};
const S: DesktopSupport = DesktopSupport {
    win10: W10,
    win11: W11,
    presence: Presence::SeparateInstall,
};
const W: DesktopSupport = DesktopSupport {
    win10: Win10Support {
        before_21h1: VersionStatus::Supported,
        from_21h1: VersionStatus::Deprecated,
    },
    win11: Win11Support {
        before_24h2: VersionStatus::Deprecated,
        from_24h2: VersionStatus::Unsupported,
    },
    presence: Presence::OptionalFeature,
};
macro_rules! x {($n:literal,$d:expr,$m:expr)=>{ExternalCommand{name:$n,aliases:&[],route:ExternalRoute::NativeExecutable,desktop:$d,modes:$m,strategy:ExternalStrategy::IdentityRaw,identity_reason:"no external adapter is released in the stable CMD increment",status:ExternalStatus::RecognizedRaw,provenance:SOURCE}};($n:literal,[$($a:literal),+],$d:expr,$m:expr)=>{ExternalCommand{name:$n,aliases:&[$($a),+],route:ExternalRoute::NativeExecutable,desktop:$d,modes:$m,strategy:ExternalStrategy::IdentityRaw,identity_reason:"no external adapter is released in the stable CMD increment",status:ExternalStatus::RecognizedRaw,provenance:SOURCE}}}
const Q: CommandModes = CommandModes::QUERY;
const QM: CommandModes = Q.union(CommandModes::MACHINE);
const QMUT: CommandModes = Q.union(CommandModes::MUTATION);
const QMUTSM: CommandModes = QMUT
    .union(CommandModes::STRUCTURED)
    .union(CommandModes::MACHINE);
const ANY: CommandModes = CommandModes::CONSERVATIVE_ANY;
macro_rules! documented {($($n:literal),* ; $($extra:expr),* $(,)?)=>{&[$(x!($n,D,ANY),)* $($extra),*]}}
pub static EXTERNAL_COMMANDS: &[ExternalCommand] = documented!("arp","attrib","auditpol","autochk","bcdboot","bdehdcfg","bitsadmin","cacls","certreq","chkdsk","chkntfs","choice","cipher","cleanmgr","clip","cmdkey","cmstp","comp","compact","convert","cscript","defrag","diantz","diskcomp","diskcopy","diskpart","diskperf","diskshadow","dispdiag","doskey","driverquery","eventcreate","expand","fc","find","findstr","fondue","forfiles","format","fsutil","ftp","fveupdate","getmac","gpresult","gpupdate","hostname","icacls","klist","label","lodctr","logman","logoff","makecab","manage-bde","mmc","mode","more","mountvol","msg","msiexec","msinfo32","mstsc","nbtstat","netcfg","netsh","netstat","nslookup","openfiles","perfmon","pktmon","pnputil","powershell","powershell_ise","print","rdpsign","recover","regini","regsvr32","relog","replace","robocopy","rpcping","rundll32","rwinsta","schtasks","secedit","setspn","setx","sfc","shadow","shutdown","sort","subst","sxstrace","systeminfo","takeown","taskkill","tasklist","timeout","tpmtool","tpmvscmgr","tracerpt","tree","tscon","tsdiscon","tskill","typeperf","tzutil","unlodctr","verifier","vssadmin","waitfor","wbadmin","wecutil","where","whoami","winrs","winsat","wscript","xcopy";
 x!("change",["chglogon","chgport","chgusr"],D,Q),x!("query",["qappsrv","qprocess","quser","qwinsta"],D,Q),
 x!("ipconfig",D,QM),x!("ping",D,QM),x!("pathping",D,QM),x!("tracert",D,QM),
 x!("net",D,ANY),x!("sc",D,ANY),x!("route",D,ANY),
 x!("reg",D,QMUTSM),x!("bcdedit",D,QMUTSM),x!("certutil",D,QMUTSM),x!("wevtutil",D,QMUTSM),
 x!("mount",O,ANY),x!("telnet",O,ANY),x!("tftp",O,QM),x!("finger",O,ANY),x!("rsh",O,ANY),x!("showmount",O,ANY),
 x!("dtrace",S,ANY),x!("pwsh",S,ANY),x!("sysmon",S,ANY),x!("wmic",W,QMUTSM));
pub fn classify_external(name: &str) -> Option<&'static ExternalCommand> {
    EXTERNAL_COMMANDS.iter().find(|e| {
        e.name.eq_ignore_ascii_case(name) || e.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    })
}
#[cfg(test)]
pub const fn external_commands() -> &'static [ExternalCommand] {
    EXTERNAL_COMMANDS
}
#[cfg(test)]
pub fn official_top_level_coverage() -> &'static [CatalogCoverage] {
    use std::sync::OnceLock;
    static C: OnceLock<Vec<CatalogCoverage>> = OnceLock::new();
    C.get_or_init(|| {
        include_str!("../../../tests/fixtures/windows_cmd/windows_commands_az.tsv")
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut f = l.split('\t');
                let source_name = f.next().unwrap();
                let normalized_name = f.next().unwrap();
                let disposition = match f.next().unwrap() {
                    "desktop-external" => CatalogDisposition::DesktopExternal,
                    "cmd-builtin" => CatalogDisposition::CmdBuiltin,
                    "unsupported-desktop" => CatalogDisposition::UnsupportedOnDesktop,
                    "optional-feature" => CatalogDisposition::OptionalDesktopFeature,
                    "separate-install" => CatalogDisposition::SeparateInstall,
                    "server-only" => CatalogDisposition::ServerOnly,
                    "subcommand-only" => CatalogDisposition::SubcommandOnly,
                    _ => panic!("bad disposition"),
                };
                assert!(f.next().is_none());
                CatalogCoverage {
                    source_name,
                    normalized_name,
                    disposition,
                    provenance: SOURCE,
                }
            })
            .collect()
    })
    .as_slice()
}
#[cfg(test)]
pub fn validate_external_manifest() -> Result<(), String> {
    let c = official_top_level_coverage();
    let mut n = std::collections::HashSet::new();
    for e in EXTERNAL_COMMANDS {
        for name in std::iter::once(e.name).chain(e.aliases.iter().copied()) {
            if !n.insert(name.to_ascii_lowercase()) {
                return Err(format!("duplicate external command name or alias: {name}"));
            }
        }
        if !matches!(e.name, "net" | "sc")
            && !c.iter().any(|r| {
                r.normalized_name.eq_ignore_ascii_case(e.name)
                    && matches!(
                        r.disposition,
                        CatalogDisposition::DesktopExternal
                            | CatalogDisposition::OptionalDesktopFeature
                            | CatalogDisposition::SeparateInstall
                    )
            })
        {
            return Err(format!("{} absent", e.name));
        }
    }
    let mut s = std::collections::HashSet::new();
    for r in c {
        if !s.insert(r.source_name.to_ascii_lowercase()) {
            return Err(format!("duplicate {}", r.source_name));
        }
        if matches!(
            r.disposition,
            CatalogDisposition::DesktopExternal
                | CatalogDisposition::OptionalDesktopFeature
                | CatalogDisposition::SeparateInstall
        ) && classify_external(r.normalized_name).is_none()
        {
            return Err(format!("{} lacks external", r.source_name));
        }
    }
    Ok(())
}
