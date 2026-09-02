#![allow(dead_code)] // Semantic output model is consumed by later adapter tasks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetClass {
    Acknowledgement,
    State,
    Collection,
    Diagnostic,
    Source,
}

impl BudgetClass {
    pub const fn max_tokens(self) -> usize {
        match self {
            Self::Acknowledgement => 128,
            Self::State => 512,
            Self::Collection => 1_024,
            Self::Diagnostic => 2_048,
            Self::Source => 4_096,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acknowledgement => "acknowledgement",
            Self::State => "state",
            Self::Collection => "collection",
            Self::Diagnostic => "diagnostic",
            Self::Source => "source",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactReason {
    Structured,
    Interactive,
    Binary,
    Streaming,
    Unknown,
    Sensitive,
}

impl ExactReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::Interactive => "interactive",
            Self::Binary => "binary",
            Self::Streaming => "streaming",
            Self::Unknown => "unknown",
            Self::Sensitive => "sensitive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputContract {
    AiOwned(BudgetClass),
    Exact(ExactReason),
    Legacy,
}

impl OutputContract {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AiOwned(_) => "ai_owned",
            Self::Exact(_) => "exact",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Success,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Success => "success",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRecord {
    pub severity: Severity,
    pub text: String,
    pub group: Option<String>,
    source_order: usize,
}

impl AiRecord {
    pub fn new(severity: Severity, text: impl Into<String>) -> Self {
        Self {
            severity,
            text: text.into(),
            group: None,
            source_order: 0,
        }
    }

    pub fn grouped(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Omission {
    pub items: usize,
    pub groups: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DocumentBody {
    Semantic,
    Legacy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiDocument {
    status: Option<String>,
    facts: Vec<(String, String)>,
    records: Vec<AiRecord>,
    body: DocumentBody,
    declared_omission: Option<Omission>,
    parser_failed: bool,
}

impl AiDocument {
    pub fn new(status: Option<impl Into<String>>) -> Self {
        Self {
            status: status.map(Into::into),
            facts: Vec::new(),
            records: Vec::new(),
            body: DocumentBody::Semantic,
            declared_omission: None,
            parser_failed: false,
        }
    }

    pub fn legacy(raw: impl Into<String>) -> Self {
        Self {
            status: None,
            facts: Vec::new(),
            records: Vec::new(),
            body: DocumentBody::Legacy(raw.into()),
            declared_omission: None,
            parser_failed: false,
        }
    }

    pub fn fact(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.facts.push((key.into(), value.into()));
    }

    pub fn push(&mut self, mut record: AiRecord) {
        record.source_order = self.records.len();
        self.records.push(record);
    }

    pub fn with_omission(mut self, omission: Omission) -> Self {
        self.declared_omission = Some(omission);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOutput {
    pub text: String,
    pub omission: Option<Omission>,
    pub parser_failed: bool,
}

#[derive(Debug)]
struct CollapsedRecord {
    group: Option<String>,
    text: String,
    source_records: usize,
}

pub fn render(document: &AiDocument, budget: BudgetClass) -> RenderedOutput {
    match &document.body {
        DocumentBody::Legacy(text) => RenderedOutput {
            text: text.clone(),
            omission: document.declared_omission.clone(),
            parser_failed: document.parser_failed,
        },
        DocumentBody::Semantic => render_semantic(document, budget),
    }
}

fn render_semantic(document: &AiDocument, budget: BudgetClass) -> RenderedOutput {
    let records = collapsed_records(document);
    let mut lines = summary_lines(document);

    if estimate_joined_tokens(&lines) > budget.max_tokens() {
        lines.clear();
    }

    let mut emitted = 0;
    for record in &records {
        let line = if record.source_records > 1 {
            format!("{} repeats={}", record.text, record.source_records)
        } else {
            record.text.clone()
        };
        let mut candidate = lines.clone();
        candidate.push(line.clone());
        if estimate_joined_tokens(&candidate) > budget.max_tokens() {
            break;
        }
        lines.push(line);
        emitted += 1;
    }

    let omission = omission_from(document.declared_omission.clone(), &records[emitted..]);

    RenderedOutput {
        text: lines.join("\n"),
        omission,
        parser_failed: document.parser_failed,
    }
}

fn summary_lines(document: &AiDocument) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(status) = &document.status {
        fields.push(format!("status={status}"));
    }
    fields.extend(
        document
            .facts
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );

    if fields.is_empty() {
        Vec::new()
    } else {
        vec![fields.join(" ")]
    }
}

fn collapsed_records(document: &AiDocument) -> Vec<CollapsedRecord> {
    let mut records = document.records.clone();
    records.sort_by_key(|record| (record.severity, record.source_order));

    let mut collapsed: Vec<CollapsedRecord> = Vec::new();
    for record in records {
        if let Some(existing) = collapsed
            .iter_mut()
            .find(|existing| existing.group == record.group && existing.text == record.text)
        {
            existing.source_records += 1;
            continue;
        }

        collapsed.push(CollapsedRecord {
            group: record.group,
            text: record.text,
            source_records: 1,
        });
    }

    collapsed
}

fn omission_from(
    declared_omission: Option<Omission>,
    omitted_records: &[CollapsedRecord],
) -> Option<Omission> {
    let mut omitted = declared_omission.unwrap_or(Omission {
        items: 0,
        groups: 0,
    });
    let mut omitted_groups = std::collections::BTreeSet::new();

    for record in omitted_records {
        omitted.items += record.source_records;
        if let Some(group) = &record.group {
            omitted_groups.insert(group);
        }
    }
    omitted.groups += omitted_groups.len();

    (omitted.items > 0 || omitted.groups > 0).then_some(omitted)
}

fn estimate_joined_tokens(lines: &[String]) -> usize {
    crate::core::tracking::estimate_tokens(&lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_limits_match_the_product_contract() {
        assert_eq!(BudgetClass::Acknowledgement.max_tokens(), 128);
        assert_eq!(BudgetClass::State.max_tokens(), 512);
        assert_eq!(BudgetClass::Collection.max_tokens(), 1_024);
        assert_eq!(BudgetClass::Diagnostic.max_tokens(), 2_048);
        assert_eq!(BudgetClass::Source.max_tokens(), 4_096);
    }

    #[test]
    fn unknown_exact_reason_is_stable_for_tracking() {
        assert_eq!(ExactReason::Unknown.as_str(), "unknown");
        assert_eq!(
            OutputContract::Exact(ExactReason::Unknown).as_str(),
            "exact"
        );
    }

    #[test]
    fn semantic_render_orders_failures_and_counts_duplicates() {
        let mut doc = AiDocument::new(Some("fail"));
        doc.fact("passed", "12");
        doc.push(AiRecord::new(Severity::Warning, "src/a.rs:2 W unused"));
        doc.push(AiRecord::new(
            Severity::Error,
            "src/b.rs:7 E0308 expected=u32 actual=String",
        ));
        doc.push(AiRecord::new(
            Severity::Error,
            "src/b.rs:7 E0308 expected=u32 actual=String",
        ));

        let rendered = render(&doc, BudgetClass::Diagnostic);

        assert_eq!(
            rendered.text,
            "status=fail passed=12\nsrc/b.rs:7 E0308 expected=u32 actual=String repeats=2\nsrc/a.rs:2 W unused"
        );
        assert_eq!(rendered.omission, None);
    }

    #[test]
    fn semantic_render_stops_before_collection_budget() {
        let mut doc = AiDocument::new(Some("ok"));
        for index in 0..300 {
            doc.push(AiRecord::new(
                Severity::Info,
                format!("src/generated/{index:03}.rs match=value"),
            ));
        }

        let rendered = render(&doc, BudgetClass::Collection);

        assert!(crate::core::tracking::estimate_tokens(&rendered.text) <= 1_024);
        assert!(rendered.omission.as_ref().is_some_and(|o| o.items > 0));
    }

    #[test]
    fn legacy_render_is_byte_compatible() {
        let raw = "native heading\n  native spacing\n";
        let rendered = render(&AiDocument::legacy(raw), BudgetClass::State);
        assert_eq!(rendered.text, raw);
        assert_eq!(rendered.omission, None);
    }
}
