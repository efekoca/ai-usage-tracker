//! CSV / JSON export of usage records and daily totals. Content-free by construction; project
//! names can be masked for sharing.

use crate::analytics::{Filter, Range};
use crate::pricing::{CostInput, PriceBook};
use crate::store::Store;
use chrono::TimeZone;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Csv,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Events,
    Daily,
}

#[derive(Debug, Serialize)]
struct Row {
    /// Local time, RFC 3339 with offset (or local date for daily rows).
    time: String,
    tool: String,
    client: String,
    model: String,
    project: String,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    reasoning: u64,
    total_tokens: u64,
    /// Empty when the model has no known price.
    api_equivalent_usd: Option<f64>,
    accuracy: String,
    events: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Writes through a temporary file next to `dest` and moves it into place only once every byte
/// is on disk, so a failed export leaves an existing file as it was.
pub fn write_atomically<T>(dest: &Path, write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> Result<T, String>) -> Result<T, String> {
    let name = dest.file_name().ok_or("export target has no file name")?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = dest.with_file_name(format!(".{}.{}-{nanos}.tmp", name.to_string_lossy(), std::process::id()));
    let result = (|| {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|e| e.to_string())?);
        let value = write(&mut f)?;
        // the last buffered write and the disk's answer are errors too; dropping would hide them
        f.into_inner().map_err(|e| e.error().to_string())?.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
        Ok(value)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub fn export<Tz: TimeZone, W: Write>(
    store: &Store,
    book: &PriceBook,
    range: Range,
    filter: &Filter,
    tz: &Tz,
    granularity: Granularity,
    format: Format,
    mask_projects: bool,
    out: &mut W,
) -> Result<usize, ExportError>
where
    Tz::Offset: std::fmt::Display,
{
    let projects: HashMap<i64, (String, bool)> =
        store.projects()?.into_iter().map(|p| (p.id, (p.name, p.hidden))).collect();
    let project_label = |id: Option<i64>| -> String {
        match id.and_then(|i| projects.get(&i).map(|p| (i, p))) {
            Some((_, (name, hidden))) if !(mask_projects || *hidden) => name.clone(),
            Some((i, _)) => format!("project-{i}"),
            None => String::new(),
        }
    };

    let mut rows: Vec<Row> = Vec::new();
    let mut daily: std::collections::BTreeMap<(String, String, String, String, String), Row> = Default::default();
    for e in store.events_between(range.from_ms, range.to_ms)? {
        if !filter.matches(&e) {
            continue;
        }
        let cost = book
            .cost(&CostInput {
                model: &e.model,
                tokens: &e.tokens,
                request_input: e.request_input,
                web_search_requests: e.web_search,
                speed: e.speed.as_deref(),
                inference_geo: e.inference_geo.as_deref(),
            })
            .map(|c| c.total());
        let local = tz.timestamp_millis_opt(e.ts_ms).single();
        let row = Row {
            time: local.as_ref().map(|d| d.to_rfc3339()).unwrap_or_default(),
            tool: e.tool.as_str().into(),
            client: e.client.clone().unwrap_or_default(),
            model: e.model.clone(),
            project: project_label(e.project_id),
            input: e.tokens.input,
            output: e.tokens.output,
            cache_read: e.tokens.cache_read,
            cache_write: e.tokens.cache_write,
            reasoning: e.tokens.reasoning,
            total_tokens: e.tokens.total(),
            api_equivalent_usd: cost,
            accuracy: e.accuracy.as_str().into(),
            events: 1,
        };
        match granularity {
            Granularity::Events => rows.push(row),
            Granularity::Daily => {
                let date = local.map(|d| d.date_naive().to_string()).unwrap_or_default();
                let key = (date.clone(), row.tool.clone(), row.model.clone(), row.project.clone(), row.accuracy.clone());
                let agg = daily.entry(key).or_insert_with(|| Row { time: date, events: 0, input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0, total_tokens: 0, api_equivalent_usd: Some(0.0), client: String::new(), ..row });
                agg.events += 1;
                agg.input += e.tokens.input;
                agg.output += e.tokens.output;
                agg.cache_read += e.tokens.cache_read;
                agg.cache_write += e.tokens.cache_write;
                agg.reasoning += e.tokens.reasoning;
                agg.total_tokens += e.tokens.total();
                agg.api_equivalent_usd = match (agg.api_equivalent_usd, cost) {
                    (Some(a), Some(c)) => Some(a + c),
                    _ => None,
                };
            }
        }
    }
    if granularity == Granularity::Daily {
        rows = daily.into_values().collect();
    }

    match format {
        Format::Json => serde_json::to_writer_pretty(&mut *out, &rows)?,
        Format::Csv => {
            writeln!(out, "time,tool,client,model,project,input,output,cache_read,cache_write,reasoning,total_tokens,api_equivalent_usd,accuracy,events")?;
            for r in &rows {
                writeln!(
                    out,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    csv(&r.time),
                    csv(&r.tool),
                    csv(&r.client),
                    csv(&r.model),
                    csv(&r.project),
                    r.input,
                    r.output,
                    r.cache_read,
                    r.cache_write,
                    r.reasoning,
                    r.total_tokens,
                    r.api_equivalent_usd.map(|c| format!("{c:.6}")).unwrap_or_default(),
                    r.accuracy,
                    r.events
                )?;
            }
        }
    }
    Ok(rows.len())
}

/// RFC 4180 quoting; also neutralises spreadsheet formula injection.
fn csv(s: &str) -> String {
    // spreadsheets skip leading blanks and accept full-width operators before a formula
    let lead = s.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
    let formula = s.starts_with(['\t', '\r']) || lead.starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠']);
    let s = if formula { format!("'{s}") } else { s.to_owned() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

#[cfg(test)]
mod tests {
    use super::csv;

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        assert_eq!(csv("a,b"), "\"a,b\"");
        assert_eq!(csv("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv("=cmd()"), "'=cmd()");
        assert_eq!(csv("plain"), "plain");
        assert_eq!(csv("  =1+1"), "'  =1+1");
        assert_eq!(csv("\u{1}@SUM(A1)"), "'\u{1}@SUM(A1)");
        assert_eq!(csv("＝cmd()"), "'＝cmd()");
        assert_eq!(csv("－1"), "'－1");
        assert_eq!(csv("\tx"), "'\tx");
        assert_eq!(csv("\rx"), "\"'\rx\"");
        assert_eq!(csv("a-b"), "a-b");
    }
}
