extern crate structopt;

use pulldown_cmark::{html, Parser};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use std::time::Duration;

use ws::Sender;

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "marp")]
struct Cli {
    #[structopt(parse(from_os_str))]
    file: PathBuf,
}

fn main() {
    let opt = Cli::from_args();
    run(&opt.file);
}

fn run(input: &PathBuf) {
    let (file_sender, file_receiver) = mpsc::channel();

    let mut watcher = watcher(file_sender, Duration::from_millis(30)).unwrap();
    watcher.watch(input, RecursiveMode::NonRecursive).unwrap();

    let initial_html = std::sync::Arc::new(parse_file(&input));

    let websocket = ws::Builder::new()
        .build(move |out: ws::Sender| {
            let cloned = initial_html.clone();

            move |_| {
                out.send(ws::Message::text(cloned.to_string())).unwrap();
                Ok(())
            }
        })
        .unwrap();

    let broadcaster = websocket.broadcaster();

    let parser_thread = thread::spawn(move || {
        print_html(file_receiver, broadcaster);
    });

    thread::spawn(move || websocket.listen("127.0.0.1:3012"));

    std::process::Command::new("open")
        .arg("src/shell.html")
        .spawn()
        .unwrap();

    parser_thread.join().unwrap();
}

fn print_html(receiver: Receiver<DebouncedEvent>, output: Sender) {
    loop {
        if let Ok(DebouncedEvent::Write(path)) = receiver.recv() {
            let markdown = parse_file(&path).to_string();
            output.send(markdown).unwrap();
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
