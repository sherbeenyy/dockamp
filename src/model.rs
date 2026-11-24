//! Plain container data + sorting/grouping/formatting. No Docker or UI types here.

use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Paused,
    Restarting,
    Created,
    Exited,
    Dead,
}

impl State {
    pub fn is_up(self) -> bool {
        matches!(self, State::Running | State::Paused | State::Restarting)
    }
}

#[derive(Clone, Debug)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: State,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
    pub exit_code: i64,
    pub health: Option<String>,
    pub restart_count: i64,
    pub command: String,
    pub ports: Vec<String>,
    pub project: Option<String>,
    /// Set when the container was created by `docker compose`; needed to rebuild it.
    pub compose: Option<Compose>,
}

#[derive(Clone, Debug)]
pub struct Compose {
    pub project: String,
    pub service: String,
    /// Project working directory (where `.env` lives).
    pub dir: String,
    pub files: Vec<String>,
}

impl ContainerInfo {
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(12)]
    }
}

/// One line of the playlist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(String),
    /// Index into the sorted container list.
    Item(usize),
}

/// Sorts containers in place (running first, then name) and builds playlist rows,
/// grouped by compose project. Ungrouped containers come under "STANDALONE",
/// and headers are only emitted if at least one project exists.
pub fn arrange(containers: &mut Vec<ContainerInfo>) -> Vec<Row> {
    let key = |c: &ContainerInfo| (!c.state.is_up(), c.name.to_lowercase());
    containers.sort_by_key(key);

    let mut groups: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for (i, c) in containers.iter().enumerate() {
        match groups.iter_mut().find(|(p, _)| *p == c.project) {
            Some((_, v)) => v.push(i),
            None => groups.push((c.project.clone(), vec![i])),
        }
    }
    let any_project = groups.iter().any(|(p, _)| p.is_some());
    // Groups with something running first, then by name; standalone last within tier.
    groups.sort_by_key(|(p, idx)| {
        let up = idx.iter().any(|&i| containers[i].state.is_up());
        (!up, p.is_none(), p.clone().unwrap_or_default().to_lowercase())
    });

    // Re-order the container vec so that display order == index order.
    let order: Vec<usize> = groups.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    let mut sorted: Vec<ContainerInfo> = order.iter().map(|&i| containers[i].clone()).collect();
    std::mem::swap(containers, &mut sorted);

    let mut rows = Vec::new();
    let mut n = 0;
    for (p, idx) in &groups {
        if any_project {
            rows.push(Row::Header(p.clone().unwrap_or_else(|| "standalone".into())));
        }
        for _ in idx {
            rows.push(Row::Item(n));
            n += 1;
        }
    }
    rows
}

/// Big LCD readout: `HH:MM:SS` below a day, `Nd HH:MM` (as `NdHH:MM`) above.
pub fn lcd_time(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d == 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}d{:02}:{:02}", d.min(99), h, m)
    }
}

/// Playlist duration column: `M:SS`, `H:MM:SS`, or `Nd HHh`.
pub fn track_time(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{}d {:02}h", d, h)
    } else if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

/// Compact "how long ago": `45s`, `12m`, `3h`, `5d`.
pub fn ago(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..=59 => format!("{}s", secs),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

pub fn bytes(b: u64) -> String {
    const K: f64 = 1024.0;
    let f = b as f64;
    if f < K * K {
        format!("{:.0}K", f / K)
    } else if f < K * K * K {
        format!("{:.0}M", f / K / K)
    } else {
        format!("{:.1}G", f / K / K / K)
    }
}

/// Truncate to `max` chars, marking the cut with `~`.
pub fn fit(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('~');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, state: State, project: Option<&str>) -> ContainerInfo {
        ContainerInfo {
            id: format!("{name}0000000000000"),
            name: name.into(),
            image: "img".into(),
            state,
            started_at: None,
            finished_at: None,
            created: None,
            exit_code: 0,
            health: None,
            restart_count: 0,
            command: String::new(),
            ports: vec![],
            project: project.map(Into::into),
            compose: None,
        }
    }

    #[test]
    fn running_first_then_alpha_no_headers() {
        let mut v = vec![
            c("zeta", State::Running, None),
            c("alpha", State::Exited, None),
            c("beta", State::Running, None),
        ];
        let rows = arrange(&mut v);
        let names: Vec<_> = v.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["beta", "zeta", "alpha"]);
        assert_eq!(rows, vec![Row::Item(0), Row::Item(1), Row::Item(2)]);
    }

    #[test]
    fn grouped_by_project() {
        let mut v = vec![
            c("solo", State::Running, None),
            c("web", State::Running, Some("shop")),
            c("db", State::Exited, Some("shop")),
            c("old", State::Exited, Some("legacy")),
        ];
        let rows = arrange(&mut v);
        let names: Vec<_> = v.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["web", "db", "solo", "old"]);
        assert_eq!(
            rows,
            vec![
                Row::Header("shop".into()),
                Row::Item(0),
                Row::Item(1),
                Row::Header("standalone".into()),
                Row::Item(2),
                Row::Header("legacy".into()),
                Row::Item(3),
            ]
        );
    }

    #[test]
    fn formatting() {
        assert_eq!(lcd_time(3661), "01:01:01");
        assert_eq!(lcd_time(86400 * 3 + 3600 * 4 + 60 * 12), "3d04:12");
        assert_eq!(track_time(75), "1:15");
        assert_eq!(track_time(3725), "1:02:05");
        assert_eq!(track_time(90000), "1d 01h");
        assert_eq!(ago(30), "30s");
        assert_eq!(ago(7200), "2h");
        assert_eq!(fit("abcdef", 4), "abc~");
        assert_eq!(fit("abc", 4), "abc");
        assert_eq!(bytes(512 * 1024 * 1024), "512M");
    }
}
