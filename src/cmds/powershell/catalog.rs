//! Offline command metadata used to decide whether a PowerShell display can be
//! compacted safely.  The list is deliberately conservative: unknown commands
//! may still use the generic structural adapter, but never receive a
//! family-specific rewrite.

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterStrategy {
    Specialized(&'static str),
    Generic,
    Identity,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKind {
    Cmdlet,
    Alias,
    Function,
    Application,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandMetadata {
    pub canonical_name: &'static str,
    pub aliases: &'static [&'static str],
    pub module: &'static str,
    pub kind: CommandKind,
    pub strategy: AdapterStrategy,
}

const ENTRIES: &[CommandMetadata] = &[
    CommandMetadata {
        canonical_name: "Get-ChildItem",
        aliases: &["gci", "ls", "dir"],
        module: "Microsoft.PowerShell.Management",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("filesystem"),
    },
    CommandMetadata {
        canonical_name: "Get-Command",
        aliases: &["gcm"],
        module: "Microsoft.PowerShell.Core",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("discovery"),
    },
    CommandMetadata {
        canonical_name: "Get-Help",
        aliases: &["help", "man"],
        module: "Microsoft.PowerShell.Core",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("discovery"),
    },
    CommandMetadata {
        canonical_name: "Get-Module",
        aliases: &["gmo"],
        module: "Microsoft.PowerShell.Core",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("discovery"),
    },
    CommandMetadata {
        canonical_name: "Get-Process",
        aliases: &["gps", "ps"],
        module: "Microsoft.PowerShell.Management",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("process"),
    },
    CommandMetadata {
        canonical_name: "Get-Service",
        aliases: &["gsv"],
        module: "Microsoft.PowerShell.Management",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("service"),
    },
    CommandMetadata {
        canonical_name: "Get-Job",
        aliases: &["gjb"],
        module: "Microsoft.PowerShell.Core",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("job"),
    },
    CommandMetadata {
        canonical_name: "Get-EventLog",
        aliases: &[],
        module: "Microsoft.PowerShell.Management",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("events"),
    },
    CommandMetadata {
        canonical_name: "Get-WinEvent",
        aliases: &[],
        module: "Microsoft.PowerShell.Diagnostics",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("events"),
    },
    CommandMetadata {
        canonical_name: "Get-CimInstance",
        aliases: &["gcim"],
        module: "CimCmdlets",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("cim"),
    },
    CommandMetadata {
        canonical_name: "Get-WmiObject",
        aliases: &["gwmi"],
        module: "Microsoft.PowerShell.Management",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("cim"),
    },
    CommandMetadata {
        canonical_name: "Get-NetAdapter",
        aliases: &[],
        module: "NetTCPIP",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("networking"),
    },
    CommandMetadata {
        canonical_name: "Get-NetIPAddress",
        aliases: &[],
        module: "NetTCPIP",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("networking"),
    },
    CommandMetadata {
        canonical_name: "Get-Volume",
        aliases: &[],
        module: "Storage",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("storage"),
    },
    CommandMetadata {
        canonical_name: "Get-Disk",
        aliases: &[],
        module: "Storage",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("storage"),
    },
    CommandMetadata {
        canonical_name: "Get-ADUser",
        aliases: &[],
        module: "ActiveDirectory",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("active-directory"),
    },
    CommandMetadata {
        canonical_name: "Get-ADComputer",
        aliases: &[],
        module: "ActiveDirectory",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("active-directory"),
    },
    CommandMetadata {
        canonical_name: "Get-VM",
        aliases: &[],
        module: "Hyper-V",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("hyper-v"),
    },
    CommandMetadata {
        canonical_name: "Get-MpComputerStatus",
        aliases: &[],
        module: "Defender",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("defender"),
    },
    CommandMetadata {
        canonical_name: "Get-BitLockerVolume",
        aliases: &[],
        module: "BitLocker",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("bitlocker"),
    },
    CommandMetadata {
        canonical_name: "Get-ScheduledTask",
        aliases: &[],
        module: "ScheduledTasks",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("scheduled-tasks"),
    },
    CommandMetadata {
        canonical_name: "Get-Container",
        aliases: &[],
        module: "Containers",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Specialized("containers"),
    },
    CommandMetadata {
        canonical_name: "Write-Output",
        aliases: &[],
        module: "Microsoft.PowerShell.Utility",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Identity,
    },
    CommandMetadata {
        canonical_name: "Write-Host",
        aliases: &[],
        module: "Microsoft.PowerShell.Utility",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Identity,
    },
    CommandMetadata {
        canonical_name: "Set-Location",
        aliases: &["cd", "sl", "chdir"],
        module: "Microsoft.PowerShell.Management",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Identity,
    },
    CommandMetadata {
        canonical_name: "Set-Variable",
        aliases: &["set"],
        module: "Microsoft.PowerShell.Utility",
        kind: CommandKind::Cmdlet,
        strategy: AdapterStrategy::Identity,
    },
];

/// Resolve a canonical command name or one of the checked-in aliases.
pub fn lookup(name: &str) -> Option<&'static CommandMetadata> {
    ENTRIES.iter().find(|entry| {
        entry.canonical_name.eq_ignore_ascii_case(name)
            || entry
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    })
}

/// Unknown profile functions and third-party commands may use only the
/// generic structural adapter after same-runspace inspection.
pub fn strategy_for(name: &str) -> AdapterStrategy {
    lookup(name)
        .map(|entry| entry.strategy)
        .unwrap_or(AdapterStrategy::Generic)
}

#[allow(dead_code)]
pub fn entries() -> &'static [CommandMetadata] {
    ENTRIES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_to_the_canonical_entry() {
        let entry = lookup("dir").expect("dir alias");
        assert_eq!(entry.canonical_name, "Get-ChildItem");
        assert_eq!(entry.strategy, AdapterStrategy::Specialized("filesystem"));
    }

    #[test]
    fn unknown_commands_are_generic_and_manifest_entries_have_strategies() {
        assert_eq!(strategy_for("My-ProfileFunction"), AdapterStrategy::Generic);
        assert_eq!(strategy_for("Write-Output"), AdapterStrategy::Identity);
        assert!(entries()
            .iter()
            .all(|entry| !entry.canonical_name.is_empty()));
    }
}
