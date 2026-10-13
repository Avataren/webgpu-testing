//! Slicing the Lua scripting guide, which is one ~6k-token Markdown file, so a caller can
//! ask for the part it needs.

/// A heading and the lines it covers: from the heading to the next heading of the same
/// or a higher level, so a section includes its subsections.
struct Section {
    level: usize,
    title: String,
    start: usize,
    end: usize,
}

/// Headings outside code fences, with the line each section ends on.
fn sections(lines: &[&str]) -> Vec<Section> {
    let mut found: Vec<Section> = Vec::new();
    let mut in_fence = false;
    for (index, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let level = line.chars().take_while(|&c| c == '#').count();
        if level == 0 || !line[level..].starts_with(' ') {
            continue;
        }
        found.push(Section {
            level,
            title: line[level..].trim().to_owned(),
            start: index,
            end: lines.len(),
        });
    }
    for index in 0..found.len() {
        let level = found[index].level;
        if let Some(next) = found[index + 1..].iter().find(|next| next.level <= level) {
            found[index].end = next.start;
        }
    }
    found
}

/// The headings, indented by level, for choosing a section.
pub fn outline(guide: &str) -> String {
    let lines: Vec<&str> = guide.lines().collect();
    sections(&lines)
        .iter()
        .map(|section| {
            format!(
                "{}{}",
                "  ".repeat(section.level.saturating_sub(1)),
                section.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every section whose title contains `query` (case-insensitive), each with its
/// subsections. A section nested in another match is not repeated.
pub fn section(guide: &str, query: &str) -> Option<String> {
    let lines: Vec<&str> = guide.lines().collect();
    let query = query.trim().to_lowercase();
    let mut taken_until = 0;
    let mut parts = Vec::new();
    for section in sections(&lines) {
        if section.start < taken_until || !section.title.to_lowercase().contains(&query) {
            continue;
        }
        parts.push(
            lines[section.start..section.end]
                .join("\n")
                .trim_end()
                .to_owned(),
        );
        taken_until = section.end;
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Lines containing `query` (case-insensitive), each under the headings that lead to it.
pub fn search(guide: &str, query: &str, limit: usize) -> String {
    let lines: Vec<&str> = guide.lines().collect();
    let sections = sections(&lines);
    let query = query.trim().to_lowercase();
    let mut hits = Vec::new();
    let mut total = 0;
    for (index, line) in lines.iter().enumerate() {
        if !line.to_lowercase().contains(&query) {
            continue;
        }
        total += 1;
        if hits.len() == limit {
            continue;
        }
        // The section path: the nearest enclosing heading at each level.
        let mut path: Vec<&str> = Vec::new();
        for section in sections
            .iter()
            .filter(|section| section.start <= index && index < section.end)
        {
            path.push(&section.title);
        }
        hits.push(format!("[{}] {}", path.join(" > "), line.trim()));
    }
    if hits.is_empty() {
        return format!("No line of the guide contains {query:?}.");
    }
    if total > hits.len() {
        hits.push(format!(
            "... {} more matches; narrow the search or ask for a section",
            total - hits.len()
        ));
    }
    hits.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUIDE: &str = "# Guide\n\nintro\n\n## Alpha\na1\n### Alpha one\nneedle here\n```\n# not a heading\n```\n## Beta\nb1\n";

    #[test]
    fn outline_ignores_headings_inside_code_fences() {
        assert_eq!(outline(GUIDE), "Guide\n  Alpha\n    Alpha one\n  Beta");
    }

    #[test]
    fn section_includes_subsections_and_stops_at_the_next_peer() {
        let alpha = section(GUIDE, "alpha").unwrap();
        assert!(alpha.starts_with("## Alpha"));
        assert!(alpha.contains("needle here") && alpha.contains("# not a heading"));
        assert!(!alpha.contains("b1"));
        assert!(section(GUIDE, "missing").is_none());
    }

    #[test]
    fn search_reports_the_heading_path() {
        assert_eq!(
            search(GUIDE, "NEEDLE", 5),
            "[Guide > Alpha > Alpha one] needle here"
        );
        assert!(search(GUIDE, "zzz", 5).starts_with("No line"));
    }
}
