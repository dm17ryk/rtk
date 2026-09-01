use std::ffi::OsString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilteredGhCommand {
    PrList,
    PrView,
    PrChecks,
    PrStatus,
    PrDiff,
    IssueList,
    IssueView,
    RunList,
    RunView,
    RepoView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GhRoute {
    Passthrough,
    Filtered {
        command: FilteredGhCommand,
        args_start: usize,
    },
}

#[derive(Clone, Copy)]
struct FlagSpec {
    long: &'static str,
    short: Option<&'static str>,
    takes_value: bool,
}

const fn value(long: &'static str, short: Option<&'static str>) -> FlagSpec {
    FlagSpec {
        long,
        short,
        takes_value: true,
    }
}

const fn switch(long: &'static str, short: Option<&'static str>) -> FlagSpec {
    FlagSpec {
        long,
        short,
        takes_value: false,
    }
}

const REPO: FlagSpec = value("--repo", Some("-R"));

const PR_LIST_FLAGS: &[FlagSpec] = &[
    value("--app", None),
    value("--assignee", Some("-a")),
    value("--author", Some("-A")),
    value("--base", Some("-B")),
    switch("--draft", Some("-d")),
    value("--head", Some("-H")),
    value("--label", Some("-l")),
    value("--limit", Some("-L")),
    value("--search", Some("-S")),
    value("--state", Some("-s")),
    REPO,
];

const PR_VIEW_FLAGS: &[FlagSpec] = &[REPO];
const PR_CHECKS_FLAGS: &[FlagSpec] = &[switch("--required", None), REPO];
const PR_STATUS_FLAGS: &[FlagSpec] = &[switch("--conflict-status", Some("-c")), REPO];
const PR_DIFF_FLAGS: &[FlagSpec] = &[value("--exclude", Some("-e")), REPO];

const ISSUE_LIST_FLAGS: &[FlagSpec] = &[
    value("--app", None),
    value("--assignee", Some("-a")),
    value("--author", Some("-A")),
    value("--label", Some("-l")),
    value("--limit", Some("-L")),
    value("--mention", None),
    value("--milestone", Some("-m")),
    value("--search", Some("-S")),
    value("--state", Some("-s")),
    value("--type", None),
    REPO,
];

const ISSUE_VIEW_FLAGS: &[FlagSpec] = &[REPO];

const RUN_LIST_FLAGS: &[FlagSpec] = &[
    switch("--all", Some("-a")),
    value("--branch", Some("-b")),
    value("--commit", Some("-c")),
    value("--created", None),
    value("--event", Some("-e")),
    value("--limit", Some("-L")),
    value("--status", Some("-s")),
    value("--user", Some("-u")),
    value("--workflow", Some("-w")),
    REPO,
];

const RUN_VIEW_FLAGS: &[FlagSpec] = &[
    value("--attempt", Some("-a")),
    switch("--exit-status", None),
    REPO,
];

const REPO_VIEW_FLAGS: &[FlagSpec] = &[value("--branch", Some("-b"))];

pub(crate) fn classify(args: &[OsString]) -> GhRoute {
    let Some(args) = args
        .iter()
        .map(|arg| arg.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return GhRoute::Passthrough;
    };

    let filtered = match args.as_slice() {
        ["pr", "list" | "ls", rest @ ..] if valid_args(rest, 0, 0, PR_LIST_FLAGS) => {
            FilteredGhCommand::PrList
        }
        ["pr", "view", rest @ ..] if valid_args(rest, 0, 1, PR_VIEW_FLAGS) => {
            FilteredGhCommand::PrView
        }
        ["pr", "checks", rest @ ..] if valid_args(rest, 0, 1, PR_CHECKS_FLAGS) => {
            FilteredGhCommand::PrChecks
        }
        ["pr", "status", rest @ ..] if valid_args(rest, 0, 0, PR_STATUS_FLAGS) => {
            FilteredGhCommand::PrStatus
        }
        ["pr", "diff", rest @ ..] if valid_args(rest, 0, 1, PR_DIFF_FLAGS) => {
            FilteredGhCommand::PrDiff
        }
        ["issue", "list" | "ls", rest @ ..]
            if valid_args(rest, 0, 0, ISSUE_LIST_FLAGS) =>
        {
            FilteredGhCommand::IssueList
        }
        ["issue", "view", rest @ ..] if valid_args(rest, 1, 1, ISSUE_VIEW_FLAGS) => {
            FilteredGhCommand::IssueView
        }
        ["run", "list" | "ls", rest @ ..] if valid_args(rest, 0, 0, RUN_LIST_FLAGS) => {
            FilteredGhCommand::RunList
        }
        ["run", "view", rest @ ..] if valid_args(rest, 1, 1, RUN_VIEW_FLAGS) => {
            FilteredGhCommand::RunView
        }
        ["repo", "view", rest @ ..] if valid_args(rest, 0, 1, REPO_VIEW_FLAGS) => {
            FilteredGhCommand::RepoView
        }
        _ => return GhRoute::Passthrough,
    };

    GhRoute::Filtered {
        command: filtered,
        args_start: 2,
    }
}

fn valid_args(
    args: &[&str],
    min_positionals: usize,
    max_positionals: usize,
    allowed_flags: &[FlagSpec],
) -> bool {
    let mut positionals = 0;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            return false;
        }

        if arg.starts_with('-') && arg != "-" {
            let (flag_name, inline_value) = arg.split_once('=').map_or((arg, None), |(name, value)| {
                (name, Some(value))
            });
            let Some(spec) = allowed_flags
                .iter()
                .find(|spec| spec.long == flag_name || spec.short == Some(flag_name))
            else {
                return false;
            };

            if spec.takes_value {
                if inline_value.is_some_and(str::is_empty) {
                    return false;
                }
                if inline_value.is_none() {
                    index += 1;
                    if index >= args.len()
                        || (args[index].starts_with('-') && args[index] != "-")
                    {
                        return false;
                    }
                }
            } else if inline_value.is_some() {
                return false;
            }
        } else {
            positionals += 1;
            if positionals > max_positionals {
                return false;
            }
        }

        index += 1;
    }

    positionals >= min_positionals
}

#[cfg(test)]
mod tests {
    use super::{classify, FilteredGhCommand, GhRoute};
    use std::ffi::OsString;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn filtered(command: FilteredGhCommand, args_start: usize) -> GhRoute {
        GhRoute::Filtered {
            command,
            args_start,
        }
    }

    #[test]
    fn classifies_only_the_supported_human_readable_routes() {
        let cases = [
            (&["pr", "list"][..], filtered(FilteredGhCommand::PrList, 2)),
            (&["pr", "ls"][..], filtered(FilteredGhCommand::PrList, 2)),
            (&["pr", "view", "42"][..], filtered(FilteredGhCommand::PrView, 2)),
            (&["pr", "checks"][..], filtered(FilteredGhCommand::PrChecks, 2)),
            (&["pr", "status"][..], filtered(FilteredGhCommand::PrStatus, 2)),
            (&["pr", "diff", "42"][..], filtered(FilteredGhCommand::PrDiff, 2)),
            (&["issue", "list"][..], filtered(FilteredGhCommand::IssueList, 2)),
            (&["issue", "ls"][..], filtered(FilteredGhCommand::IssueList, 2)),
            (&["issue", "view", "42"][..], filtered(FilteredGhCommand::IssueView, 2)),
            (&["run", "list"][..], filtered(FilteredGhCommand::RunList, 2)),
            (&["run", "ls"][..], filtered(FilteredGhCommand::RunList, 2)),
            (&["run", "view", "12345"][..], filtered(FilteredGhCommand::RunView, 2)),
            (&["repo", "view"][..], filtered(FilteredGhCommand::RepoView, 2)),
        ];

        for (args, expected) in cases {
            assert_eq!(classify(&argv(args)), expected, "args: {args:?}");
        }
    }

    #[test]
    fn accepts_documented_selector_flags_for_filtered_routes() {
        let cases = [
            (&["pr", "list", "--author", "@me", "--draft", "-R", "o/r"][..], FilteredGhCommand::PrList),
            (&["pr", "view", "feature", "--repo=o/r"][..], FilteredGhCommand::PrView),
            (&["pr", "checks", "42", "--required", "-R", "o/r"][..], FilteredGhCommand::PrChecks),
            (&["pr", "status", "--conflict-status", "--repo", "o/r"][..], FilteredGhCommand::PrStatus),
            (&["pr", "diff", "42", "--exclude", "generated/*", "-R", "o/r"][..], FilteredGhCommand::PrDiff),
            (&["issue", "list", "--label", "bug", "--limit=50", "-R", "o/r"][..], FilteredGhCommand::IssueList),
            (&["issue", "view", "42", "--repo", "o/r"][..], FilteredGhCommand::IssueView),
            (&["run", "list", "--all", "-w", "ci.yml", "--limit", "50", "-R", "o/r"][..], FilteredGhCommand::RunList),
            (&["run", "view", "12345", "--attempt", "2", "--exit-status", "-R", "o/r"][..], FilteredGhCommand::RunView),
            (&["repo", "view", "o/r", "--branch", "main"][..], FilteredGhCommand::RepoView),
        ];

        for (args, command) in cases {
            assert_eq!(classify(&argv(args)), filtered(command, 2), "args: {args:?}");
        }
    }

    #[test]
    fn accepts_every_safe_selector_spelling() {
        let cases: &[(&[&str], &[&str], FilteredGhCommand)] = &[
            (&["pr", "list"], &["--app", "dependabot"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--assignee", "@me"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-a", "@me"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--author", "@me"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-A", "@me"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--base", "main"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-B", "main"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--draft"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-d"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--head", "feature"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-H", "feature"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--label", "bug"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-l", "bug"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--limit", "50"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-L", "50"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--search", "review:required"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-S", "review:required"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["--state", "all"], FilteredGhCommand::PrList),
            (&["pr", "list"], &["-s", "all"], FilteredGhCommand::PrList),
            (&["pr", "view", "42"], &["--repo", "o/r"], FilteredGhCommand::PrView),
            (&["pr", "checks", "42"], &["--required"], FilteredGhCommand::PrChecks),
            (&["pr", "status"], &["--conflict-status"], FilteredGhCommand::PrStatus),
            (&["pr", "status"], &["-c"], FilteredGhCommand::PrStatus),
            (&["pr", "diff", "42"], &["--exclude", "generated/*"], FilteredGhCommand::PrDiff),
            (&["pr", "diff", "42"], &["-e", "generated/*"], FilteredGhCommand::PrDiff),
            (&["issue", "list"], &["--app", "dependabot"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--assignee", "@me"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-a", "@me"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--author", "@me"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-A", "@me"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--label", "bug"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-l", "bug"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--limit", "50"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-L", "50"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--mention", "octocat"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--milestone", "v1"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-m", "v1"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--search", "no:assignee"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-S", "no:assignee"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--state", "all"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["-s", "all"], FilteredGhCommand::IssueList),
            (&["issue", "list"], &["--type", "Bug"], FilteredGhCommand::IssueList),
            (&["issue", "view", "42"], &["-R", "o/r"], FilteredGhCommand::IssueView),
            (&["run", "list"], &["--all"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-a"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--branch", "main"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-b", "main"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--commit", "abc123"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-c", "abc123"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--created", "today"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--event", "push"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-e", "push"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--limit", "50"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-L", "50"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--status", "failure"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-s", "failure"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--user", "@me"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-u", "@me"], FilteredGhCommand::RunList),
            (&["run", "list"], &["--workflow", "ci.yml"], FilteredGhCommand::RunList),
            (&["run", "list"], &["-w", "ci.yml"], FilteredGhCommand::RunList),
            (&["run", "view", "123"], &["--attempt", "2"], FilteredGhCommand::RunView),
            (&["run", "view", "123"], &["-a", "2"], FilteredGhCommand::RunView),
            (&["run", "view", "123"], &["--exit-status"], FilteredGhCommand::RunView),
            (&["repo", "view", "o/r"], &["--branch", "main"], FilteredGhCommand::RepoView),
            (&["repo", "view", "o/r"], &["-b", "main"], FilteredGhCommand::RepoView),
        ];

        for (prefix, flag_args, command) in cases {
            let mut args = prefix.to_vec();
            args.extend_from_slice(flag_args);
            assert_eq!(classify(&argv(&args)), filtered(*command, 2), "args: {args:?}");
        }
    }

    #[test]
    fn malformed_known_flags_passthrough_without_reinterpretation() {
        for args in [
            &["pr", "list", "--author"][..],
            &["pr", "list", "--author", "--web"][..],
            &["run", "list", "--limit", "--status", "failure"][..],
            &["run", "view", "123", "--attempt", "--exit-status"][..],
            &["pr", "list", "--draft=true"][..],
            &["pr", "checks", "--required=true"][..],
        ] {
            assert_eq!(classify(&argv(args)), GhRoute::Passthrough, "args: {args:?}");
        }
    }

    #[test]
    fn exact_interactive_mutating_streaming_and_unknown_modes_passthrough() {
        let cases: &[&[&str]] = &[
            &[],
            &["--help"],
            &["--version"],
            &["pr", "list", "--json", "number,title"],
            &["pr", "list", "--json=number,title"],
            &["pr", "list", "--jq", ".[].number"],
            &["pr", "view", "42", "--template", "{{.title}}"],
            &["pr", "view", "42", "--web"],
            &["pr", "list", "--web"],
            &["pr", "list", "--future-flag"],
            &["pr", "view", "42", "--comments"],
            &["pr", "checks", "--help"],
            &["pr", "checks", "42", "--watch"],
            &["pr", "checks", "42", "--interval", "5"],
            &["pr", "checks", "42", "--fail-fast"],
            &["pr", "checks", "42", "--web"],
            &["pr", "checks", "42", "--json", "name,state"],
            &["pr", "checks", "42", "--jq", ".[].name"],
            &["pr", "checks", "42", "--template", "{{.name}}"],
            &["pr", "diff", "42", "--patch"],
            &["pr", "diff", "42", "--name-only"],
            &["pr", "diff", "42", "--color", "always"],
            &["pr", "diff", "42", "--allow-escape-sequences"],
            &["pr", "diff", "42", "--no-compact"],
            &["issue", "list", "--web"],
            &["issue", "view"],
            &["issue", "view", "42", "--comments"],
            &["issue", "view", "42", "--web"],
            &["run", "view", "--help"],
            &["run", "view"],
            &["run", "view", "12345", "--job", "456"],
            &["run", "view", "12345", "--log"],
            &["run", "view", "12345", "--log-failed"],
            &["run", "view", "12345", "--verbose"],
            &["run", "view", "12345", "--web"],
            &["run", "view", "12345", "--json", "jobs"],
            &["run", "view", "12345", "--jq", ".jobs"],
            &["run", "view", "12345", "--template", "{{.name}}"],
            &["repo", "view", "--web"],
            &["pr", "create", "--title", "title"],
            &["pr", "edit", "42", "--title", "title"],
            &["pr", "comment", "42", "--body", "body"],
            &["pr", "merge", "42"],
            &["auth", "login"],
            &["api", "repos/o/r"],
            &["alias", "expand", "co"],
            &["co", "42"],
            &["my-extension", "command", "--future-flag"],
        ];

        for args in cases {
            assert_eq!(classify(&argv(args)), GhRoute::Passthrough, "args: {args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_arguments_passthrough() {
        use std::os::unix::ffi::OsStringExt;

        let args = vec![OsString::from("pr"), OsString::from_vec(vec![0xff])];
        assert_eq!(classify(&args), GhRoute::Passthrough);
    }
}
