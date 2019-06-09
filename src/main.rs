extern crate serde;
extern crate structopt;

use pulldown_cmark::{html, Parser};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use serde::Serialize;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use std::time::Duration;

use ws::Sender;

use structopt::StructOpt;

const DEFAULT_STYLES: &str = include_str!("default.css");

#[derive(Clone, Debug, StructOpt)]
#[structopt(name = "marp")]
struct Cli {
    #[structopt(parse(from_os_str))]
    file: PathBuf,
    #[structopt(short = "s", long = "stylesheet", parse(from_os_str))]
    stylesheet: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct Update {
    stylesheet: Option<String>,
    content: Option<String>,
}

fn main() {
    let opt = Cli::from_args();
    run(opt);
}

fn run(opt: Cli) {
    let input = &opt.file;


    let styles = if let Some(stylesheet_path) = &opt.stylesheet {
        std::fs::read_to_string(&stylesheet_path).expect("could not read file")
    } else {
        DEFAULT_STYLES.to_string()
    };

    let shared_styles = std::sync::Arc::new(styles);
    let initial_html = std::sync::Arc::new(parse_file(&input));


    let websocket = ws::Builder::new()
        .build(move |out: ws::Sender| {
            let cloned_content = initial_html.clone();
            let cloned_styles = shared_styles.clone();

            move |_| {
                let initial_message = Update {
                    content: Some(cloned_content.to_string()),
                    stylesheet: Some(cloned_styles.to_string()),
                };

                let serialized = serde_json::to_string(&initial_message).unwrap();
                out.send(ws::Message::text(serialized.to_string())).unwrap();
                println!("Connection established");
                Ok(())
            }
        })
        .unwrap();

    let broadcaster = websocket.broadcaster();

    let cli = opt.clone();
    let parser_thread = thread::spawn(move || {
        watch_and_parse(&cli, broadcaster);
    });

    thread::spawn(move || websocket.listen("127.0.0.1:3012"));

    std::process::Command::new("open")
        .arg("src/shell.html")
        .spawn()
        .unwrap();

    parser_thread.join().unwrap();
}

fn watch_and_parse(config: &Cli, output: Sender) {
    let (sender, receiver) = mpsc::channel();

    let mut watcher = watcher(sender, Duration::from_millis(30)).unwrap();
    watcher
        .watch(&config.file, RecursiveMode::NonRecursive)
        .unwrap();

    if let Some(stylesheet) = &config.stylesheet {
        watcher
            .watch(stylesheet, RecursiveMode::NonRecursive)
            .unwrap();
    }

    let canonical_content_path = std::fs::canonicalize(&config.file).unwrap();
    let canonical_stylesheet_path = config
        .stylesheet
        .as_ref()
        .map(|p| std::fs::canonicalize(p).unwrap());

    loop {
        if let Ok(DebouncedEvent::Write(path)) = receiver.recv() {
            let (content, stylesheet) = if path == canonical_content_path {
                (Some(parse_file(&path).to_string()), None)
            } else if Some(&path) == canonical_stylesheet_path.as_ref() {
                let styles = std::fs::read_to_string(&path).expect("could not read file");
                (None, Some(styles))
            } else {
                unreachable!();
            };

            let update = Update {
                content,
                stylesheet,
            };

            let serialized = serde_json::to_string(&update).unwrap();
            output.send(serialized).unwrap();
        }
    }
}

fn parse_file(path: &PathBuf) -> String {
    let content = std::fs::read_to_string(&path).expect("could not read file");
    let parser = Parser::new(&content);

    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}
