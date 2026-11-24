//! Background Docker worker: owns the bollard client on a tokio runtime thread and
//! talks to the UI over channels. Event-driven: one initial listing, then container
//! events trigger re-inspects. Only the selected container gets a stats stream.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use bollard::container::{
    InspectContainerOptions, ListContainersOptions, RestartContainerOptions,
    StartContainerOptions, Stats, StatsOptions, StopContainerOptions,
};
use bollard::models::{ContainerInspectResponse, ContainerStateStatusEnum};
use bollard::system::EventsOptions;
use bollard::Docker;
use chrono::{DateTime, Datelike, Utc};
use futures_util::StreamExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::model::{ContainerInfo, State};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Start,
    Stop,
    Pause,
    Unpause,
    Restart,
}

impl Action {
    pub fn progress(self) -> &'static str {
        match self {
            Action::Start => "STARTING...",
            Action::Stop => "STOPPING...",
            Action::Pause => "PAUSING...",
            Action::Unpause => "RESUMING...",
            Action::Restart => "RESTARTING...",
        }
    }
    fn done(self) -> &'static str {
        match self {
            Action::Start => "STARTED",
            Action::Stop => "STOPPED",
            Action::Pause => "PAUSED",
            Action::Unpause => "RESUMED",
            Action::Restart => "RESTARTED",
        }
    }
}

pub enum Command {
    Select(Option<String>),
    Act(String, Action),
}

#[derive(Debug, Clone)]
pub struct StatsSample {
    pub id: String,
    /// Percent of one core (docker convention: can exceed 100).
    pub cpu: f32,
    pub mem: u64,
    pub mem_limit: u64,
}

pub enum Update {
    Snapshot(Vec<ContainerInfo>),
    Stats(StatsSample),
    ActionDone(String),
    Error(String),
}

/// Spawns the worker thread. `notify` is called after every update so the UI repaints.
pub fn spawn(
    tx: Sender<Update>,
    notify: impl Fn() + Send + Sync + 'static,
) -> UnboundedSender<Command> {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("docker".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let out = Out { tx, notify: std::sync::Arc::new(notify) };
            rt.block_on(run(cmd_rx, out));
        })
        .expect("spawn docker thread");
    cmd_tx
}

#[derive(Clone)]
struct Out {
    tx: Sender<Update>,
    notify: std::sync::Arc<dyn Fn() + Send + Sync>,
}

impl Out {
    fn send(&self, u: Update) {
        let _ = self.tx.send(u);
        (self.notify)();
    }
}

async fn run(mut cmd_rx: UnboundedReceiver<Command>, out: Out) {
    let mut selected: Option<String> = None;
    let mut stats_task: Option<JoinHandle<()>> = None;

    loop {
        let docker = match Docker::connect_with_defaults() {
            Ok(d) => d,
            Err(e) => {
                out.send(Update::Error(format!("CANNOT CONNECT TO DOCKER: {e}")));
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };

        let mut map: HashMap<String, ContainerInfo> = HashMap::new();
        if let Err(e) = load_all(&docker, &mut map).await {
            out.send(Update::Error(format!("CANNOT CONNECT TO DOCKER: {e}")));
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }
        out.send(Update::Snapshot(map.values().cloned().collect()));
        restart_stats(&docker, &out, &selected, &mut stats_task);

        let mut filters = HashMap::new();
        filters.insert("type".to_string(), vec!["container".to_string()]);
        let mut events = Box::pin(docker.events(Some(EventsOptions {
            filters,
            ..Default::default()
        })));

        loop {
            tokio::select! {
                ev = events.next() => match ev {
                    Some(Ok(ev)) => {
                        let action = ev.action.unwrap_or_default();
                        // Health checks spam exec_* events; they don't change state.
                        if action.starts_with("exec_") { continue; }
                        let Some(id) = ev.actor.and_then(|a| a.id) else { continue };
                        refresh_one(&docker, &mut map, &id).await;
                        out.send(Update::Snapshot(map.values().cloned().collect()));
                        if selected.as_deref() == Some(id.as_str()) {
                            restart_stats(&docker, &out, &selected, &mut stats_task);
                        }
                    }
                    _ => {
                        out.send(Update::Error("LOST DOCKER CONNECTION, RETRYING".into()));
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                },
                cmd = cmd_rx.recv() => match cmd {
                    None => return, // UI closed
                    Some(Command::Select(id)) => {
                        if id != selected {
                            selected = id;
                            restart_stats(&docker, &out, &selected, &mut stats_task);
                        }
                    }
                    Some(Command::Act(id, action)) => {
                        let (docker, out) = (docker.clone(), out.clone());
                        tokio::spawn(async move {
                            let res = match action {
                                Action::Start => docker.start_container(&id, None::<StartContainerOptions<String>>).await,
                                Action::Stop => docker.stop_container(&id, None::<StopContainerOptions>).await,
                                Action::Pause => docker.pause_container(&id).await,
                                Action::Unpause => docker.unpause_container(&id).await,
                                Action::Restart => docker.restart_container(&id, None::<RestartContainerOptions>).await,
                            };
                            let msg = match res {
                                Ok(()) => action.done().to_string(),
                                Err(e) => format!("ERROR: {}", short_err(&e)),
                            };
                            out.send(Update::ActionDone(msg));
                        });
                    }
                },
            }
        }
    }
}

fn short_err(e: &bollard::errors::Error) -> String {
    match e {
        bollard::errors::Error::DockerResponseServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

async fn load_all(
    docker: &Docker,
    map: &mut HashMap<String, ContainerInfo>,
) -> Result<(), bollard::errors::Error> {
    let list = docker
        .list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() }))
        .await?;
    map.clear();
    for s in list {
        if let Some(id) = s.id {
            refresh_one(docker, map, &id).await;
        }
    }
    Ok(())
}

async fn refresh_one(docker: &Docker, map: &mut HashMap<String, ContainerInfo>, id: &str) {
    match docker.inspect_container(id, None::<InspectContainerOptions>).await {
        Ok(r) => {
            if let Some(info) = convert(r) {
                map.insert(info.id.clone(), info);
            }
        }
        // Gone (destroyed) — or briefly unavailable; either way drop it.
        Err(_) => {
            map.remove(id);
        }
    }
}

fn parse_time(s: Option<String>) -> Option<DateTime<Utc>> {
    let t = DateTime::parse_from_rfc3339(&s?).ok()?.with_timezone(&Utc);
    // Docker uses 0001-01-01 for "never".
    (t.year() > 1971).then_some(t)
}

fn convert(r: ContainerInspectResponse) -> Option<ContainerInfo> {
    let id = r.id?;
    let st = r.state.unwrap_or_default();
    let state = match st.status {
        Some(ContainerStateStatusEnum::RUNNING) => State::Running,
        Some(ContainerStateStatusEnum::PAUSED) => State::Paused,
        Some(ContainerStateStatusEnum::RESTARTING) => State::Restarting,
        Some(ContainerStateStatusEnum::CREATED) => State::Created,
        Some(ContainerStateStatusEnum::DEAD) => State::Dead,
        _ => State::Exited,
    };
    let config = r.config.unwrap_or_default();
    let labels = config.labels.unwrap_or_default();

    let mut command = r.path.unwrap_or_default();
    for a in r.args.unwrap_or_default() {
        command.push(' ');
        command.push_str(&a);
    }

    let mut ports: Vec<String> = Vec::new();
    if let Some(pm) = r.network_settings.and_then(|n| n.ports) {
        for (cport, binds) in pm {
            match binds {
                Some(b) if !b.is_empty() => {
                    for pb in b {
                        let ip = pb.host_ip.unwrap_or_default();
                        // Skip the IPv6 twin of an IPv4 binding.
                        if ip == "::" { continue; }
                        ports.push(format!("{}:{}->{}", ip, pb.host_port.unwrap_or_default(), cport));
                    }
                }
                _ => ports.push(cport),
            }
        }
    }
    ports.sort();
    ports.dedup();

    Some(ContainerInfo {
        id,
        name: r.name.unwrap_or_default().trim_start_matches('/').to_string(),
        image: config.image.unwrap_or_default(),
        state,
        started_at: parse_time(st.started_at),
        finished_at: parse_time(st.finished_at),
        created: parse_time(r.created),
        exit_code: st.exit_code.unwrap_or(0),
        health: st
            .health
            .and_then(|h| h.status)
            .map(|s| format!("{s:?}").to_lowercase())
            .filter(|s| s != "none" && s != "empty"),
        restart_count: r.restart_count.unwrap_or(0),
        command,
        ports,
        project: labels.get("com.docker.compose.project").cloned(),
    })
}

fn restart_stats(
    docker: &Docker,
    out: &Out,
    selected: &Option<String>,
    task: &mut Option<JoinHandle<()>>,
) {
    if let Some(t) = task.take() {
        t.abort();
    }
    let Some(id) = selected.clone() else { return };
    let (docker, out) = (docker.clone(), out.clone());
    *task = Some(tokio::spawn(async move {
        let mut s = docker.stats(&id, Some(StatsOptions { stream: true, one_shot: false }));
        while let Some(Ok(st)) = s.next().await {
            out.send(Update::Stats(sample(&id, &st)));
        }
    }));
}

fn sample(id: &str, s: &Stats) -> StatsSample {
    let cpu_delta = s.cpu_stats.cpu_usage.total_usage as f64
        - s.precpu_stats.cpu_usage.total_usage as f64;
    let sys_delta = s.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
        - s.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;
    let cpus = s
        .cpu_stats
        .online_cpus
        .or_else(|| s.cpu_stats.cpu_usage.percpu_usage.as_ref().map(|v| v.len() as u64))
        .unwrap_or(1) as f64;
    let cpu = if cpu_delta > 0.0 && sys_delta > 0.0 {
        (cpu_delta / sys_delta * cpus * 100.0) as f32
    } else {
        0.0
    };
    StatsSample {
        id: id.to_string(),
        cpu,
        mem: s.memory_stats.usage.unwrap_or(0),
        mem_limit: s.memory_stats.limit.unwrap_or(0),
    }
}

