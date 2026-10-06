use clap::Parser;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};
use woomy_ai::{AiProvider, AiService, ContextBuilder};
use woomy_core::{AnalysisService, ProjectService};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "woomy",
    version,
    about = "Wii U static analysis CLI. Uses an existing authorized local dump."
)]
struct Args {
    #[arg(long, global = true, default_value = ".")]
    project: PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    no_color: bool,
    #[arg(
        long,
        global = true,
        help = "Confirm project creation (never configures AI)"
    )]
    yes: bool,
    #[arg(long, global = true)]
    no_ai: bool,
    #[arg(long, global = true, default_value_t = 100)]
    limit: usize,
    #[arg(long, global = true, default_value_t = 0)]
    offset: usize,
    #[arg(num_args=0..)]
    command: Vec<String>,
}
#[derive(Debug)]
struct AppError {
    code: u8,
    message: String,
}
type Result<T> = std::result::Result<T, AppError>;
impl From<woomy_core::Error> for AppError {
    fn from(e: woomy_core::Error) -> Self {
        Self {
            code: e.exit_code(),
            message: e.to_string(),
        }
    }
}
impl From<woomy_ai::Error> for AppError {
    fn from(e: woomy_ai::Error) -> Self {
        Self {
            code: 5,
            message: e.to_string(),
        }
    }
}
impl From<io::Error> for AppError {
    fn from(e: io::Error) -> Self {
        Self {
            code: 4,
            message: e.to_string(),
        }
    }
}
fn config(message: &str) -> AppError {
    AppError {
        code: 2,
        message: message.into(),
    }
}
fn main() -> std::process::ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            if args.json {
                println!("{}", json!({"error":e.message,"exit_code":e.code}));
            } else {
                eprintln!("Error: {}", clean(&e.message));
            }
            std::process::ExitCode::from(e.code)
        }
    }
}
fn run(args: &Args) -> Result<()> {
    let cancel = Arc::new(AtomicU8::new(0));
    let flag = cancel.clone();
    ctrlc::set_handler(move || {
        if flag.swap(1, Ordering::SeqCst) != 0 {
            std::process::exit(6);
        }
        eprintln!("Requesting graceful cancellation... Press Ctrl+C again to force exit.");
    })
    .map_err(|_| config("cannot install Ctrl+C handler"))?;
    if args.command.first().is_some_and(|s| s == "help") {
        print_help(args.command.get(1).map(String::as_str), args.json);
        return Ok(());
    }
    let mut project = if ProjectService::exists(&args.project) {
        if args.command.is_empty() && !args.json {
            eprintln!("Loading project...");
        }
        ProjectService::open(&args.project)?
    } else {
        if !args.command.is_empty() && args.command[0] != "setup" {
            return Err(AppError {
                code: 3,
                message:
                    "No project. Run woomy setup --yes --no-ai, or launch woomy interactively."
                        .into(),
            });
        }
        setup(args, &cancel)?
    };
    let mut ai = AiService::default();
    if !args.command.is_empty() {
        let value = dispatch(&mut project, &mut ai, &args.command, args, &cancel)?;
        render(&value, args.json);
        return Ok(());
    }
    if args.json {
        return Err(config("--json requires a command"));
    }
    println!("============================================================\n Woomy\n Project: {}\n============================================================",clean(&project.config.project.name));
    let status = project.status()?;
    render(&status, false);
    if status["last_run"].is_object() {
        println!("Previous analysis found. Use runs for checkpoint history.");
    }
    if project.interrupted()?
        && project.config.analysis.auto_resume
        && io::stdin().is_terminal()
        && yes("Previous analysis was interrupted. Resume? [Y/n] ", true)?
    {
        render(
            &dispatch(&mut project, &mut ai, &["resume".into()], args, &cancel)?,
            false,
        );
    }
    println!("Ready. Type \"help\" for commands.");
    shell(project, ai, args, cancel)
}
fn prompt(message: &str) -> Result<String> {
    print!("{message}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Err(AppError {
            code: 6,
            message: "Input closed; cancelled".into(),
        });
    }
    if line.len() > 4096 {
        return Err(config("input too long"));
    }
    Ok(line.trim().into())
}
fn yes(message: &str, default: bool) -> Result<bool> {
    let s = prompt(message)?;
    Ok(if s.is_empty() {
        default
    } else {
        s.eq_ignore_ascii_case("y") || s.eq_ignore_ascii_case("yes")
    })
}
fn setup(args: &Args, cancel: &AtomicU8) -> Result<ProjectService> {
    if !ProjectService::detect(&args.project) {
        return Err(AppError {code:3,message:"No Wii U dump was detected. Expected code/, content/, meta/. Place Woomy in your authorized dump root or use --project <dump-root>.".into()});
    }
    if args.json && !args.yes {
        return Err(config("setup --json requires --yes --no-ai"));
    }
    if !args.yes {
        println!("============================================================\n Woomy First-Time Setup\n Wii U Static Recompiler\n============================================================\nNo Woomy project was found in this directory.\nDump detected: code/ content/ meta/");
        let m = ProjectService::metadata(&args.project)?;
        println!(
            "Game: {}\nTitle ID: {}\nRegion (metadata mask): {}\nVersion: {}",
            clean(m.name.as_deref().unwrap_or("Unknown")),
            clean(m.title_id.as_deref().unwrap_or("Unknown")),
            clean(m.region.as_deref().unwrap_or("Unknown")),
            clean(m.version.as_deref().unwrap_or("Unknown"))
        );
        if !yes("Create a Woomy project here? [Y/n] ", true)? {
            return Err(AppError {
                code: 6,
                message: "Setup cancelled".into(),
            });
        }
    }
    let mut p = ProjectService::create(&args.project)?;
    if !args.json {
        eprintln!("Project configuration and SQLite database created.");
    }
    if !args.yes && !args.no_ai && yes("Configure AI now? [y/S] ", false)? {
        match setup_ai(&mut p) {
            Ok(value) => render(&value, false),
            Err(e) => eprintln!("{}", clean(&e.message)),
        }
    }
    p.scan(cancel, |s| eprintln!("{s}"))?;
    if !args.json {
        eprintln!("Scan complete. Ready.");
    }
    Ok(p)
}
fn setup_ai(project: &mut ProjectService) -> Result<Value> {
    if !io::stdin().is_terminal() {
        return Err(config("ai setup requires a terminal for masked key entry; use environment configuration in automation"));
    }
    println!("Provider: [1] OpenAI [2] Configure later");
    if prompt("> ")? != "1" {
        return Ok(json!({"ai":"configuration skipped"}));
    }
    let model = prompt("OpenAI model ID (configurable, no default): ")?;
    let scope = woomy_ai::credential_scope(project);
    let key = if std::env::var_os("OPENAI_API_KEY").is_some()
        && yes("Use OPENAI_API_KEY from this environment? [Y/n] ", true)?
    {
        woomy_ai::load_key(&scope)?
    } else {
        let key = zeroize::Zeroizing::new(rpassword::prompt_password("API key (hidden): ")?);
        woomy_ai::save_key(&scope, &key)?;
        key
    };
    project.config.ai.provider = "openai".into();
    project.config.ai.model = model;
    project.config.ai.enabled = true;
    project.save_config()?;
    println!("Testing AI connection (model access; no inference request)...");
    woomy_ai::OpenAiProvider::new(key, &project.config.ai.model)?.test_connection()?;
    Ok(
        json!({"provider":"OpenAI","connection":"Connected","model":project.config.ai.model,"note":"Model access verified; generation is tested by ai analyze."}),
    )
}
fn model_profile(value: Option<&str>) -> Result<Option<bool>> {
    match value {None=>Ok(None),Some("debugger-absent")=>Ok(Some(false)),Some("debugger-present")=>Ok(Some(true)),_=>Err(config("profile must be debugger-absent or debugger-present; omission keeps process state UNKNOWN"))}
}
fn argument<'a>(words: &'a [String], n: usize, usage: &str) -> Result<&'a str> {
    words
        .get(n)
        .map(String::as_str)
        .ok_or_else(|| config(usage))
}
fn address(s: &str) -> Result<u32> {
    u32::from_str_radix(
        s.strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(s),
        16,
    )
    .map_err(|_| config("address must be hexadecimal"))
}
fn dispatch(
    project: &mut ProjectService,
    ai: &mut AiService,
    words: &[String],
    args: &Args,
    cancel: &AtomicU8,
) -> Result<Value> {
    let cmd = argument(words, 0, "missing command")?;
    if !["scan", "analyze", "resume", "status", "setup"].contains(&cmd) {
        project.refresh_module_hashes()?;
    }
    match cmd {
        "exit" | "quit" => Ok(Value::Null),
        "status" | "setup" => Ok(project.status()?),
        "scan" => Ok(json!(project.scan(cancel, |s| eprintln!("{s}"))?)),
        "analyze" | "resume" => Ok(json!(AnalysisService::run(
            project,
            cancel,
            |s| eprintln!("{s}")
        )?)),
        "validate-function" => {
            if words.get(2).map(String::as_str) != Some("synthetic") || words.len() != 3 {
                return Err(config("validate-function <function-id> synthetic: explicit opt-in for 512 bounded synthetic register states"));
            }
            Ok(project.validate_function_synthetic(argument(
                words,
                1,
                "validate-function <function-id> synthetic",
            )?)?)
        }
        "image" => Ok(project.image_query(
            argument(
                words,
                1,
                "image status|regions|relocations|verify|indirect-audit|link-audit [module-id]",
            )?,
            words.get(2).map(String::as_str),
            args.limit,
            args.offset,
        )?),
        "semantic" => Ok(project.semantic_analysis(words.get(1).map(String::as_str))?),
        "context" => Ok(project.execution_context(argument(words, 1, "context <function-id>")?)?),
        "closure" => Ok(project.function_closure(argument(words, 1, "closure <function-id>")?)?),
        "entry" => match argument(
            words,
            1,
            "entry frontier|run-model debugger-absent|debugger-present",
        )? {
            "frontier" => Ok(project.entry_frontier()?),
            "run-model" => Ok(project.run_entry_model(
                model_profile(words.get(2).map(String::as_str))?.ok_or_else(|| {
                    config("entry run-model requires debugger-absent or debugger-present")
                })?,
            )?),
            _ => Err(config(
                "entry frontier|run-model debugger-absent|debugger-present",
            )),
        },
        "imports" if words.get(1).map(String::as_str) == Some("catalog") => {
            Ok(project.import_catalog()?)
        }
        "snapshot" => {
            if words.get(1).map(String::as_str) == Some("replay") {
                return Ok(project.replay_snapshot(argument(
                    words,
                    2,
                    "snapshot replay <.woomy-relative-file>",
                )?)?);
            }
            let command = argument(words, 1, "snapshot inspect|validate <.woomy-relative-file>")?;
            if !["inspect", "validate"].contains(&command) {
                return Err(config("snapshot inspect|validate <.woomy-relative-file>"));
            }
            Ok(project.inspect_snapshot(
                argument(words, 2, "snapshot inspect|validate <.woomy-relative-file>")?,
                command == "validate",
            )?)
        }
        "modules" | "imports" | "exports" | "problems" | "unknown" | "runs" | "indirect"
        | "jump_tables" => Ok(json!(project.list(
            cmd,
            words.get(1).map(String::as_str).unwrap_or(""),
            args.limit,
            args.offset
        )?)),
        "module" => {
            let id = argument(words, 1, "module <module-id>")?;
            Ok(
                json!({"module":project.list("modules",id,1,0)?,"sections":project.list("sections",id,args.limit,args.offset)?}),
            )
        }
        "functions" => Ok(json!(project.list(
            cmd,
            words.get(1).map(String::as_str).unwrap_or(""),
            args.limit,
            args.offset
        )?)),
        "function" => {
            let f = argument(words, 1, "function <id>")?;
            Ok(
                json!({"function":project.require_function(f)?,"evidence":project.list("evidence",f,args.limit,args.offset)?}),
            )
        }
        "disasm" | "cfg" | "dataflow" | "values" | "definitions" | "dataflow_status"
        | "indirect_targets" => {
            let f = argument(words, 1, "command <function-id>")?;
            project.require_function(f)?;
            if cmd == "cfg" {
                Ok(
                    json!({"blocks":project.list("blocks",f,args.limit,args.offset)?,"edges":project.list("cfg",f,args.limit,args.offset)?}),
                )
            } else {
                Ok(json!(project.list(cmd, f, args.limit, args.offset)?))
            }
        }
        "xref" => {
            let direction = argument(words, 1, "xref to|from <hex-address>")?;
            let value = address(argument(words, 2, "xref to|from <hex-address>")?)?;
            let kind = match direction {
                "to" => "xref_to",
                "from" => "xref_from",
                _ => return Err(config("xref to|from <hex-address>")),
            };
            Ok(json!(project.list(
                kind,
                &value.to_string(),
                args.limit,
                args.offset
            )?))
        }
        "config" => {
            if words.len() == 1 {
                return Ok(json!(project.config));
            }
            if words.get(1).map(String::as_str) != Some("performance") {
                return Err(config("config performance eco|balanced|maximum"));
            }
            let profile = argument(words, 2, "config performance eco|balanced|maximum")?;
            if !["eco", "balanced", "maximum"].contains(&profile) {
                return Err(config("invalid performance profile"));
            }
            if project.config.analysis.performance != profile {
                project.config.analysis.performance = profile.into();
                project.save_config()?;
                project
                    .db
                    .execute("UPDATE modules SET status='STALE'", [])
                    .map_err(woomy_core::Error::from)?;
            }
            Ok(
                json!({"performance":profile,"workers":1,"cache":"Reanalysis required after profile change"}),
            )
        }
        "ai" => ai_command(project, ai, words, args),
        "pause" => Ok(json!({"status":"IDLE","message":"No analysis is currently running"})),
        "cancel" => {
            project.mark_cancelled()?;
            Ok(json!({"status":"CANCELLED","message":"Committed checkpoints preserved"}))
        }
        "clear" => {
            if io::stdout().is_terminal() {
                print!("\x1b[2J\x1b[H");
            }
            Ok(Value::Null)
        }
        "help" => {
            print_help(words.get(1).map(String::as_str), args.json);
            Ok(Value::Null)
        }
        "ir" if words.get(1).map(String::as_str) == Some("coverage") => Ok(project.ir_coverage()?),
        "codegen" | "build" => {
            let status = words.get(1).map(String::as_str) == Some("status");
            let id = argument(
                words,
                if status { 2 } else { 1 },
                "codegen|build [status] <function-id>",
            )?;
            if status {
                Ok(project.codegen_status(id)?)
            } else if cmd == "codegen" {
                Ok(project.codegen(id)?)
            } else {
                Ok(project.build_native(id)?)
            }
        }
        "ir" => Ok(project.inspect_ir(
            argument(words, 1, "ir <function-id>")?,
            args.limit,
            args.offset,
        )?),
        "calls" => Ok(project.calls(
            argument(words, 1, "calls <function-id>")?,
            None,
            args.limit,
            args.offset,
        )?),
        "call" => Ok(project.calls(
            argument(words, 1, "call <function-id> <hex-site>")?,
            Some(address(argument(
                words,
                2,
                "call <function-id> <hex-site>",
            )?)?),
            args.limit,
            args.offset,
        )?),
        "summary" => Ok(project.function_summary(argument(words, 1, "summary <function-id>")?)?),
        "runtime" => {
            match argument(words, 1, "runtime status|imports")? {
                "connect" => Ok(project.connect_manmenmi(std::path::Path::new(argument(
                    words,
                    2,
                    "runtime connect <Manmenmi-M10-source-directory>",
                )?))?),
                "process" => Ok(project
                    .runtime_process(model_profile(words.get(2).map(String::as_str))?, false)?),
                "thread" => Ok(project
                    .runtime_process(model_profile(words.get(2).map(String::as_str))?, true)?),
                "status" => Ok(project.runtime_status()?),
                "imports" => Ok(project.runtime_imports(args.limit, args.offset)?),
                _ => Err(config("runtime status|imports")),
            }
        }
        "native" if words.get(1).is_some_and(|s| s == "status") => {
            Ok(project.native_status(argument(words, 2, "native status <function-id>")?)?)
        }
        "verify-ir" => Ok(project.verify_ir(argument(words, 1, "verify-ir <function-id>")?)?),
        "compile" | "recompile" => Err(config("NOT IMPLEMENTED")),
        _ => Err(config("unknown command; type help")),
    }
}
fn ai_command(
    project: &mut ProjectService,
    ai: &mut AiService,
    words: &[String],
    args: &Args,
) -> Result<Value> {
    match words.get(1).map(String::as_str).unwrap_or("status") {
        "status" => Ok(
            json!({"enabled":project.config.ai.enabled,"provider":project.config.ai.provider,"model":project.config.ai.model,"connection":"Not tested in this command","requests_session":ai.requests,"max_requests_session":project.config.ai.max_requests_session,"permissions":["READ","PROPOSE"]}),
        ),
        "setup" => setup_ai(project),
        "test" => {
            woomy_ai::provider(project)?.test_connection()?;
            Ok(json!({"connection":"Connected","generation":"Not tested"}))
        }
        "model" => {
            if let Some(m) = words.get(2) {
                if m.len() > 128 || m.is_empty() {
                    return Err(config("invalid model ID"));
                }
                project.config.ai.model = m.clone();
                project.save_config()?;
            }
            Ok(json!({"model":project.config.ai.model}))
        }
        "provider" => {
            let p = argument(words, 2, "ai provider openai|mock")?;
            if !["openai", "mock"].contains(&p) {
                return Err(config("unsupported provider"));
            }
            project.config.ai.provider = p.into();
            project.save_config()?;
            Ok(json!({"provider":p}))
        }
        "enable" | "disable" => {
            project.config.ai.enabled = words[1] == "enable";
            project.save_config()?;
            Ok(json!({"enabled":project.config.ai.enabled}))
        }
        "context" => {
            let f = argument(words, 2, "ai context <function-id>")?;
            let c = ContextBuilder::build(project, f, project.config.ai.max_context_bytes)?;
            Ok(serde_json::from_str(&c.json).map_err(woomy_core::Error::from)?)
        }
        "analyze" | "explain" => {
            let f = argument(words, 2, "ai analyze <function-id>")?;
            eprintln!(
                "Building bounded context; sending to configured provider. Output is AI_PROPOSED."
            );
            let provider = woomy_ai::provider(project)?;
            Ok(ai.analyze(project, f, provider.as_ref())?)
        }
        "unresolved" => Ok(
            json!({"indirect":project.list("unresolved","",args.limit,args.offset)?,"unknown":project.list("unknown","",args.limit,args.offset)?,"problems":project.list("problems","",args.limit,args.offset)?}),
        ),
        "findings" => Ok(json!(project.list(
            "findings",
            "",
            args.limit,
            args.offset
        )?)),
        "finding" => {
            let id = argument(words, 2, "ai finding <id>")?;
            let n = id
                .parse::<i64>()
                .map_err(|_| config("invalid finding ID"))?;
            Ok(
                json!({"finding":project.list("findings",id,1,0)?,"evidence":project.rows("SELECT description FROM ai_finding_evidence WHERE finding=?1 ORDER BY ordinal",&[&n])?}),
            )
        }
        "tools" => Ok(
            json!({"tools":woomy_ai::ReadTools::NAMES,"search_strings":"NOT IMPLEMENTED","remote_tool_loop":"NOT IMPLEMENTED"}),
        ),
        "run" => Err(config(
            "Autonomous AI scheduling NOT IMPLEMENTED; use ai analyze <function-id>",
        )),
        "stop" => Ok(json!({"status":"IDLE","message":"No autonomous AI job is running"})),
        _ => Err(config("unknown AI command; help ai")),
    }
}

type Job = thread::JoinHandle<(ProjectService, Result<Value>)>;
fn shell(
    project: ProjectService,
    mut ai: AiService,
    args: &Args,
    cancel: Arc<AtomicU8>,
) -> Result<()> {
    let (sender, receiver) = mpsc::channel();
    let (read_sender, read_receiver) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        while read_receiver.recv().is_ok() {
            let mut line = String::new();
            let result = stdin.lock().read_line(&mut line);
            match result {
                Ok(0) => break,
                Ok(_) => {
                    if sender.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = sender.send(Err(e));
                    break;
                }
            }
        }
    });
    let mut project = Some(project);
    let mut job: Option<Job> = None;
    let mut exiting = false;
    print!("woomy> ");
    io::stdout().flush()?;
    let _ = read_sender.send(());
    loop {
        if job.as_ref().is_some_and(|j| j.is_finished()) {
            if let Some(j) = job.take() {
                let (p, result) = j.join().map_err(|_| AppError {
                    code: 4,
                    message: "Analysis worker failed. Restart Woomy to recover its checkpoint."
                        .into(),
                })?;
                project = Some(p);
                match result {
                    Ok(v) => render(&v, false),
                    Err(e) => eprintln!("{}", clean(&e.message)),
                };
                cancel.store(0, Ordering::SeqCst);
                if !exiting {
                    print!("woomy> ");
                    io::stdout().flush()?;
                }
            }
        }
        if exiting && job.is_none() {
            break;
        }
        if exiting {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let line = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(l) => l?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                exiting = true;
                cancel.store(1, Ordering::SeqCst);
                continue;
            }
        };
        if line.len() > 4096 {
            eprintln!("Command too long");
            let _ = read_sender.send(());
            continue;
        }
        let Some(words) = shlex::split(line.trim_end_matches(['\r', '\n'])) else {
            eprintln!("Unclosed quote");
            let _ = read_sender.send(());
            continue;
        };
        if words.is_empty() {
            print!("woomy> ");
            io::stdout().flush()?;
            let _ = read_sender.send(());
            continue;
        }
        if words[0] == "exit" || words[0] == "quit" {
            exiting = true;
            cancel.store(1, Ordering::SeqCst);
            continue;
        }
        if job.is_some() {
            match words[0].as_str(){"pause"=>{cancel.store(2,Ordering::SeqCst);println!("Pause requested at next safe checkpoint.");},"cancel"=>{cancel.store(1,Ordering::SeqCst);println!("Cancellation requested.");},"status"=>println!("Analysis RUNNING; committed checkpoints remain recoverable."),"help"=>print_help(words.get(1).map(String::as_str),false),_=>eprintln!("Analysis is running. Use status, pause, cancel or exit; inspect results after it stops.")}
        } else if ["scan", "analyze", "resume"].contains(&words[0].as_str()) {
            let Some(mut p) = project.take() else {
                return Err(config("project unavailable"));
            };
            let a = args.clone();
            let c = cancel.clone();
            cancel.store(0, Ordering::SeqCst);
            job = Some(thread::spawn(move || {
                let result = dispatch(&mut p, &mut AiService::default(), &words, &a, &c);
                (p, result)
            }));
        } else if let Some(p) = project.as_mut() {
            match dispatch(p, &mut ai, &words, args, &cancel) {
                Ok(v) => render(&v, false),
                Err(e) => eprintln!("{}", clean(&e.message)),
            }
        }
        print!("woomy> ");
        io::stdout().flush()?;
        let _ = read_sender.send(());
    }
    Ok(())
}
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '?'
            } else {
                c
            }
        })
        .collect()
}
fn render(value: &Value, json_mode: bool) {
    if value.is_null() {
        return;
    }
    if json_mode {
        println!("{value}");
    } else if value.get("database").is_some() && value.get("project").is_some() {
        for (label, key) in [
            ("Project", "project"),
            ("Database", "database"),
            ("Dump", "dump"),
            ("Files", "files"),
            ("Modules", "modules"),
            ("Stale modules", "stale_modules"),
            ("RPX/RPL", "rpx"),
            ("PPC", "ppc"),
            ("Functions", "functions"),
            ("Instructions", "instructions"),
            ("Unknown PPC", "unknown_instructions"),
            ("Blocks", "blocks"),
            ("CFG", "cfg"),
            ("Dataflow", "dataflow"),
            ("Indirect unresolved", "unresolved_indirect"),
            ("IR", "ir"),
            ("C++", "cpp"),
        ] {
            let item = &value[key];
            let s = item
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| item.to_string());
            println!("{label:<20} {}", clean(&s));
        }
        println!(
            "{:<20} {}",
            "AI",
            if value["ai"]["enabled"] == true {
                "Enabled (connection not tested)"
            } else {
                "Disabled"
            }
        );
    } else if let Ok(s) = serde_json::to_string_pretty(value) {
        println!("{}", clean(&s));
    }
}
fn print_help(command: Option<&str>, json_mode: bool) {
    let commands=[("semantic","semantic [function-id]: derive and persist static semantic hypotheses, with evidence and denominators"),("entry","entry frontier | entry run-model debugger-absent|debugger-present: static paths or bounded PPC/IR/native M10 model comparison"),("validate-function","validate-function <function-id> synthetic: opt-in, 512 bounded states; no real game context claim"),("image","image status|regions|relocations|verify|indirect-audit|link-audit [module-id]: derived linked-address image"),("context","context <function-id>: provenance and missing execution inputs"),("closure","closure <function-id>: bounded known dependencies and blockers"),("snapshot","snapshot inspect|validate|replay <.woomy-relative-file>: bounded provenance checks and explicit replay"),("calls","calls <function-id>: versioned targets and conservative guest effects"),("call","call <function-id> <hex-site>: inspect a guest call contract"),("summary","summary <function-id>: fixed-point may-effects and stack observations"),("runtime","runtime status|imports|process|thread [debugger-absent|debugger-present]; runtime connect <M10-source-directory>"),("native","native status <function-id>: compile state and guest execution prerequisites"),("help","help [command]"),("status","status: database, dump change check, counts and honest capability status"),("setup","setup [--yes --no-ai]: create .woomy only in an existing dump root"),("scan","scan: hash every dump file; originals are read only"),("analyze","analyze: scan, parse, reachable PPC, CFG, calls, xrefs and liveness"),("modules","modules: list stable module IDs"),("module","module <id>: module and section metadata"),("functions","functions [search]: inferred entries from recorded evidence"),("function","function <id>: metadata and evidence"),("disasm","disasm <function-id>: structured PPC instructions"),("cfg","cfg <function-id>: blocks and intraprocedural edges"),("dataflow","dataflow <function-id>: conservative register liveness"),("values","values <function-id>: inferred scalar and symbolic register values"),("definitions","definitions <function-id>: reaching register definitions"),("dataflow_status","dataflow_status <function-id>: convergence and budgets"),("indirect","indirect [function-id]: CTR/LR sites, returns, resolution status and evidence"),("indirect_targets","indirect_targets <function-id>: static/imported targets and proof"),("jump_tables","jump_tables [function-id]: bounded switch candidates and evidence"),("codegen","codegen [status] <function-id>: verified IR to C++; unsupported effects rejected"),("build","build [status] <function-id>: MSVC compile/link only; never executes game functions"),("ir","ir coverage | <function-id>: typed machine IR, paginated by blocks; unknown effects remain explicit"),("verify-ir","verify-ir <function-id>: verify structure/types/provenance, not execution equivalence"),("xref","xref to|from <hex-address>"),("imports","imports [module-id]"),("exports","exports [module-id]"),("problems","problems: errors and analysis limitations"),("unknown","unknown: unknown/invalid opcode statistics; relocation dependencies are counted separately in status"),("runs","runs: checkpoint history"),("ai","ai status|setup|test|provider openai|mock|model [id]|enable|disable|context <function>|analyze <function>|explain <function>|unresolved|findings|finding <id>|tools|run|stop. run and remote tool loop NOT IMPLEMENTED. API requests send bounded context; findings stay AI_PROPOSED."),("config","config [performance eco|balanced|maximum]: bounded single worker"),("pause","pause: request shell analysis stop at safe checkpoint"),("resume","resume: rehash, reuse compatible completed modules, retry remaining modules"),("cancel","cancel: cancel active analysis, preserve committed checkpoints"),("clear","clear: clear compatible terminal"),("exit","exit: graceful cancellation and close")];
    let items: Vec<_> = commands
        .iter()
        .filter(|(name, _)| command.is_none_or(|c| c == *name))
        .map(|(name, help)| json!({"command":name,"help":help}))
        .collect();
    if json_mode {
        println!("{}", json!({"commands":items}));
    } else {
        for item in items {
            println!("{}", item["help"].as_str().unwrap_or(""));
        }
        println!("Command mode: --project <dump-root> --json --no-color --limit 1..1000 --offset N. Shell list pages use the launch limits.");
    }
}
