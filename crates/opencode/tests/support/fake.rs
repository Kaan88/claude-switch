use std::{env, fs, io::Read, process, thread, time::Duration};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mode = env::var("FAKE_MODE").unwrap_or_default();
    if !args.starts_with(&["--session-id"])
        && let Ok(log) = env::var("FAKE_LOG")
    {
        use std::io::Write;
        let mut out = fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
        writeln!(out, "{}", args.join(" ")).unwrap();
    }
    match args.as_slice() {
        ["--version"] => println!("  2.0.5  "),
        ["service", "status"] if mode == "down" => println!("OpenCode service is not running"),
        ["service", "status"] => println!("http://127.0.0.1:12345"),
        ["api", "session.active"] if mode == "malformed" => println!(r#"{{"data":[]}}"#),
        ["api", "session.active"] if mode == "unknown" => println!(r#"{{"data":{{"a":{{"type":"future"}}}}}}"#),
        ["api", "session.active"] if mode == "idle" => println!(r#"{{"data":{{}}}}"#),
        ["api", "session.active"] => println!(
            r#"{{"data":{{"a":{{"type":"running"}},"b":{{"type":"idle"}},"c":{{"type":"retry"}},"d":{{"type":"running"}}}}}}"#
        ),
        ["service", "restart"] if matches!(mode.as_str(), "fallback" | "failed-stop") => process::exit(1),
        ["service", "stop"] if mode == "failed-stop" => process::exit(1),
        ["service", "restart" | "start" | "stop"] => {}
        ["models"] => println!("z/a\np/group/model#high\np/first\np/first\nnot-a-model\np/"),
        ["run", "--model", "p/model", ..] if mode == "private_error" => {
            eprintln!("private diagnostic");
            process::exit(1);
        }
        ["sleep"] | ["run", "--model", "p/model", "--sleep"] => {
            let done = std::path::PathBuf::from(env::var_os("FAKE_DONE").unwrap());
            fs::write(done.with_extension("started"), "").unwrap();
            thread::sleep(Duration::from_secs(2));
            fs::write(done, "not killed").unwrap();
        }
        ["run", "--model", "p/model", rest @ ..] => {
            assert!(!rest.iter().any(|arg| arg.contains("private marker")));
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            print!("{input}");
        }
        ["reload"] if mode == "error" => {
            eprintln!(" reload failure ");
            process::exit(3);
        }
        ["reload"] => println!(" reloaded "),
        ["--session-id", "fake-id", rest @ ..] => {
            let (name_arg, rest) = rest.split_first().unwrap();
            let name = name_arg.strip_prefix("--name=").expect("name is one native option with its value");
            assert_eq!(name, env::var("FAKE_CLAUDE_NAME").unwrap());
            let (agent, first) = match rest {
                ["--", first] => (None, first),
                ["--agent", agent, "--", first] => (Some(*agent), first),
                _ => panic!("wrong agent or prompt argument placement"),
            };
            assert_eq!(agent, env::var("FAKE_CLAUDE_AGENT").ok().as_deref());
            let text = if let Some(path) = first
                .strip_prefix("Read the handover file at \"")
                .and_then(|s| s.strip_suffix("\" in full before doing anything else. Then continue from it."))
            {
                assert!(std::path::Path::new(path).is_absolute());
                assert!(!first.contains("long private marker"));
                fs::read_to_string(path).unwrap()
            } else {
                (*first).into()
            };
            fs::write(env::var_os("FAKE_CLAUDE_OUT").unwrap(), text).unwrap();
        }
        _ => panic!("unexpected fake arguments"),
    }
}
