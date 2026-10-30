//! Charts and SmartArt, read for display: a chart's type, title and the
//! data Word caches inside the chart part; the text of a SmartArt diagram.
//! docex never changes them.

use crate::xml::{Document, Element};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub name: String,
    pub values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    /// column, bar, line, area, pie, doughnut, scatter, or other.
    pub kind: String,
    pub title: String,
    pub categories: Vec<String>,
    pub series: Vec<Series>,
}

fn text_of(e: &Element) -> String {
    let mut s = String::new();
    e.visit(&mut |x| {
        if x.is("a:t") || x.is("c:v") {
            s.push_str(&x.text_content());
        }
    });
    s.trim().to_string()
}

/// Cached points (`c:pt idx` / `c:v`) of a reference or literal, by index.
/// Multi-level categories give their innermost level (Word stores it first).
fn points(e: Option<&Element>) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let Some(e) = e else { return out };
    let mut levels = Vec::new();
    e.descendants("c:lvl", &mut levels);
    let src = levels.first().copied().unwrap_or(e);
    let mut pts = Vec::new();
    src.descendants("c:pt", &mut pts);
    for x in pts {
        let idx = x.attr("idx").and_then(|i| i.parse().ok()).unwrap_or(out.len());
        out.push((idx, x.child("c:v").map(|v| v.text_content()).unwrap_or_default()));
    }
    out
}

fn dense(pts: Vec<(usize, String)>) -> Vec<String> {
    let n = pts.iter().map(|(i, _)| i + 1).max().unwrap_or(0);
    let mut v = vec![String::new(); n];
    for (i, s) in pts {
        v[i] = s;
    }
    v
}

pub fn parse_chart(doc: &Document) -> Option<Chart> {
    let chart = doc.root.child("c:chart")?;
    let plot = chart.child("c:plotArea")?;
    let mut out = Chart::default();
    if chart.child("c:autoTitleDeleted").and_then(|a| a.attr("val")) != Some("1") {
        if let Some(t) = chart.child("c:title") {
            out.title = text_of(t);
        }
    }
    for group in plot.elements() {
        let kind = match group.local() {
            "barChart" | "bar3DChart" => {
                if group.child("c:barDir").and_then(|d| d.attr("val")) == Some("bar") { "bar" } else { "column" }
            }
            "lineChart" | "line3DChart" | "stockChart" => "line",
            "areaChart" | "area3DChart" => "area",
            "pieChart" | "pie3DChart" | "ofPieChart" => "pie",
            "doughnutChart" => "doughnut",
            "scatterChart" | "bubbleChart" => "scatter",
            "radarChart" | "surfaceChart" | "surface3DChart" => "other",
            _ => continue,
        };
        if out.kind.is_empty() {
            out.kind = kind.to_string();
        }
        for ser in group.elements().filter(|s| s.is("c:ser")) {
            let name = ser.child("c:tx").map(text_of).unwrap_or_default();
            let cats = ser.child("c:cat").or_else(|| ser.child("c:xVal"));
            if out.categories.is_empty() {
                out.categories = dense(points(cats));
            }
            let vals = ser.child("c:val").or_else(|| ser.child("c:yVal"));
            let values = dense(points(vals)).iter().map(|v| v.trim().parse::<f64>().ok()).collect();
            out.series.push(Series { name, values });
        }
    }
    if out.kind.is_empty() {
        return None;
    }
    let n = out.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if out.categories.len() < n {
        out.categories.extend((out.categories.len() + 1..=n).map(|i| i.to_string()));
    }
    Some(out)
}

/// The text of a SmartArt diagram's nodes, in order (`dgm:dataModel`).
pub fn smartart_text(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    doc.root.visit(&mut |x| {
        if x.is("dgm:pt") && matches!(x.attr("type"), None | Some("node")) {
            if let Some(t) = x.child("dgm:t") {
                let s = text_of(t);
                if !s.is_empty() {
                    out.push(s);
                }
            }
        }
    });
    out
}

impl Chart {
    /// A readable summary: title and kind, then a pipe table of the data.
    pub fn summary(&self) -> String {
        let fmt = |v: &Option<f64>| match v {
            Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", *x as i64),
            Some(x) => format!("{x}"),
            None => String::new(),
        };
        let mut s = format!("{} chart", self.kind);
        if !self.title.is_empty() {
            s = format!("{}: {s}", self.title);
        }
        s.push_str(&format!("\n| | {} |", self.categories.join(" | ")));
        for ser in &self.series {
            let vals: Vec<String> = ser.values.iter().map(fmt).collect();
            s.push_str(&format!("\n| {} | {} |", if ser.name.is_empty() { "values" } else { &ser.name }, vals.join(" | ")));
        }
        s
    }
}
