//! Worker-side log following and interactive shells.

use std::time::Duration;

use bollard::container::{LogOutput, LogsOptions};
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use bollard::Docker;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{UnboundedReceiver, WeakUnboundedSender};

use crate::console::{lock, LogBuf, Shared, Term, COLS, ROWS};
use crate::docker::short_err;

const TAIL: &str = "1000";

/// Announces the shell's PID (so it can be hung up later), then becomes the best shell around.
const LAUNCH: &str = r#"printf '\033]777;dockamp=%s\007' "$$"
if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi"#;

/// Fills `buf` with a container's recent and new log output until it stops.
pub async fn follow_logs(docker: Docker, id: String, buf: Shared<LogBuf>, notify: impl Fn()) {
    lock(&buf).clear();
    let opts = LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        tail: TAIL.into(),
        ..Default::default()
    };
    let mut stream = docker.logs(&id, Some(opts)).ready_chunks(512);
    while let Some(batch) = stream.next().await {
        {
            let mut b = lock(&buf);
            for item in batch {
                match item {
                    Ok(out) => {
                        let stderr = matches!(out, LogOutput::StdErr { .. });
                        b.push(&out.into_bytes(), stderr);
                    }
                    Err(e) => b.note(&short_err(&e)),
                }
            }
        }
        notify();
        // Caps the repaint rate; Docker holds the rest back.
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    lock(&buf).finish();
    notify();
}

/// Runs an interactive shell until it exits or the UI drops its end of `input`.
pub async fn shell(
    docker: Docker,
    id: String,
    term: Shared<Term>,
    input: UnboundedReceiver<Vec<u8>>,
    reply: WeakUnboundedSender<Vec<u8>>,
    notify: impl Fn(),
) {
    let msg = match run_shell(&docker, &id, &term, input, reply, &notify).await {
        Ok(msg) => msg,
        Err(e) => short_err(&e),
    };
    lock(&term).ended = Some(msg);
    notify();
}

async fn run_shell(
    docker: &Docker,
    id: &str,
    term: &Shared<Term>,
    mut input: UnboundedReceiver<Vec<u8>>,
    reply: WeakUnboundedSender<Vec<u8>>,
    notify: &impl Fn(),
) -> Result<String, bollard::errors::Error> {
    let exec = docker
        .create_exec(
            id,
            CreateExecOptions {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(true),
                // The default, Ctrl+P Ctrl+Q, would swallow Ctrl+P (previous command).
                detach_keys: Some("ctrl-^,ctrl-_"),
                env: Some(vec!["TERM=xterm-256color"]),
                cmd: Some(vec!["sh", "-c", LAUNCH]),
                ..Default::default()
            },
        )
        .await?
        .id;
    let opts = StartExecOptions { tty: true, ..Default::default() };
    let StartExecResults::Attached { output, input: mut stdin } = docker.start_exec(&exec, Some(opts)).await? else {
        return Ok("DETACHED".into());
    };

    let resize = async {
        let size = ResizeExecOptions { height: ROWS as u16, width: COLS as u16 };
        // Fails until the process has really started.
        for _ in 0..20 {
            if docker.resize_exec(&exec, size).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let read = async {
        let mut output = output.ready_chunks(64);
        while let Some(batch) = output.next().await {
            let mut answers = Vec::new();
            {
                let mut t = lock(term);
                for out in batch.into_iter().flatten() {
                    answers.extend(t.process(&out.into_bytes()));
                }
            }
            if let (false, Some(tx)) = (answers.is_empty(), reply.upgrade()) {
                let _ = tx.send(answers);
            }
            notify();
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    };
    // Separate from reading, so a big paste can't deadlock against its own echo.
    let write = async {
        while let Some(bytes) = input.recv().await {
            if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                // The read side reports the end of the session.
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        _ = async { tokio::join!(resize, read) } => {
            let code = docker.inspect_exec(&exec).await.ok().and_then(|r| r.exit_code);
            Ok(match code {
                Some(126 | 127) if lock(term).pid().is_none() => "NO SHELL IN THIS CONTAINER".into(),
                Some(c) => format!("SHELL EXITED ({c})"),
                None => "SHELL EXITED".into(),
            })
        }
        _ = write => {
            hang_up(docker, id, term).await;
            Ok("CLOSED".into())
        }
    }
}

/// Docker leaves exec'd processes running when their client goes away, so do what closing a
/// terminal does: SIGHUP everything in the shell's session (daemons have left it by then).
async fn hang_up(docker: &Docker, id: &str, term: &Shared<Term>) {
    let Some(pid) = lock(term).pid() else { return };
    let script = format!(
        r#"for d in /proc/[0-9]*; do
  read -r s 2>/dev/null <"$d/stat" || continue
  set -- ${{s##*) }}
  [ "$4" = {pid} ] && kill -HUP "${{d#/proc/}}" && kill -CONT "${{d#/proc/}}"
done 2>/dev/null"#
    );
    let opts = CreateExecOptions { cmd: Some(vec!["sh", "-c", script.as_str()]), ..Default::default() };
    if let Ok(exec) = docker.create_exec(id, opts).await {
        let start = StartExecOptions { detach: true, ..Default::default() };
        let _ = docker.start_exec(&exec.id, Some(start)).await;
    }
}
