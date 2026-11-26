use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn read(pipe: impl Read + Send + 'static, limit: u64) -> mpsc::Receiver<Result<Vec<u8>, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe.take(limit + 1).read_to_end(&mut bytes);
        let _ = tx.send(match result {
            Ok(_) if bytes.len() as u64 <= limit => Ok(bytes),
            _ => Err("OpenCode output could not be read or was too large.".into()),
        });
    });
    rx
}

pub(super) fn capture(
    mut command: Command,
    input: Option<&str>,
    private: bool,
    cancelled: impl Fn() -> bool,
    timeout: Duration,
) -> Result<Output, String> {
    if cancelled() {
        return Err("Cancelled.".into());
    }
    let started = Instant::now();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(if private { Stdio::null() } else { Stdio::piped() })
        .spawn()
        .map_err(
            |e| if private { "Could not start OpenCode.".into() } else { format!("Could not run OpenCode: {e}") },
        )?;
    let limit = if private { 1024 * 1024 } else { 16 * 1024 * 1024 };
    let out = read(child.stdout.take().expect("piped stdout"), limit);
    let err = child.stderr.take().map(|pipe| read(pipe, limit));
    let (tx, rx) = mpsc::channel();
    let mut sent = input.is_none();
    if let Some((mut pipe, text)) = child.stdin.take().zip(input) {
        let input = text.as_bytes().to_vec();
        std::thread::spawn(move || {
            let result = pipe.write_all(&input);
            drop(pipe);
            let _ = tx.send(result);
        });
    }
    let mut status = None;
    let result = (|| {
        let mut stdout = None;
        let mut stderr = private.then(Vec::new);
        loop {
            if cancelled() {
                return Err("Cancelled.".into());
            }
            if started.elapsed() >= timeout {
                return Err("OpenCode timed out.".into());
            }
            if status.is_none() {
                status = child.try_wait().map_err(|_| "Could not wait for OpenCode.")?;
            }
            if !sent {
                match rx.try_recv() {
                    Ok(Ok(())) => sent = true,
                    Err(mpsc::TryRecvError::Empty) => {}
                    _ => return Err("OpenCode did not read the full request.".into()),
                }
            }
            collect(&out, &mut stdout)?;
            if let Some(err) = &err {
                collect(err, &mut stderr)?;
            }
            if let Some(status) = status
                && sent
                && stdout.is_some()
                && stderr.is_some()
            {
                return Ok(Output { status, stdout: stdout.unwrap_or_default(), stderr: stderr.unwrap_or_default() });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn collect(rx: &mpsc::Receiver<Result<Vec<u8>, String>>, target: &mut Option<Vec<u8>>) -> Result<(), String> {
    if target.is_none() {
        match rx.try_recv() {
            Ok(result) => *target = Some(result?),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => return Err("Could not read OpenCode output.".into()),
        }
    }
    Ok(())
}
