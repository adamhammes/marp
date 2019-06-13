extern crate hyper;
extern crate liquid;
extern crate serde;
extern crate structopt;


use hyper::rt::Future;
use hyper::service::service_fn_ok;
use hyper::{Body, Response, Server};
use pulldown_cmark::{html, Parser};
use std::path::PathBuf;

use serde::Serialize;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use std::time::Duration;

use ws::Sender;

use structopt::StructOpt;

const DEFAULT_STYLES: &str = include_str!("default.css");

const WEB_TEMPLATE: &str = include_str!("shell.html");

#[derive(Clone, Debug, StructOpt)]
#[structopt(name = "marp")]
struct Cli {
    #[structopt(parse(from_os_str))]
    file: PathBuf,
    #[structopt(
        short = "s",
        long = "stylesheet",
        help = "A .css file to replace the default styles",
        parse(from_os_str)
    )]
    stylesheet: Option<PathBuf>,
    #[structopt(
        long = "no-open",
        help = "Do not open the rendered markdown in the browser"
    )]
    no_open: bool,
    #[structopt(short = "p", long = "port", default_value = "8000")]
    port: u16,
}

#[derive(Debug, Serialize)]
struct Update {
    stylesheet: Option<String>,
    content: Option<String>,
}

fn main() {
    run(Cli::from_args());
}

fn run(opt: Cli) {
    let styles = match &opt.stylesheet {
        Some(path) => std::fs::read_to_string(&path).expect("could not read file"),
        None => DEFAULT_STYLES.to_string(),
    };

    let initial_html = parse_file(&opt.file);

    let websocket = build_websocket(initial_html, styles);
    let broadcaster = websocket.broadcaster();

    let rendered_template = std::sync::Arc::new(render_web_template());

    let addr = ([127, 0, 0, 1], opt.port).into();
    let server = Server::bind(&addr)
        .serve(move || {
            let cloned = rendered_template.clone();
            service_fn_ok(move |_| Response::new(Body::from(cloned.to_string())))
        })
        .map_err(|e| eprintln!("server error: {}", e));

    let open = !&opt.no_open;
    crossbeam::scope(|scope| {
        scope.spawn(move |_| watch_and_parse(&opt, broadcaster));
        scope.spawn(move |_| websocket.listen("127.0.0.1:3012"));
        scope.spawn(move |_| hyper::rt::run(server));

        println!("Serving content at http://{}", addr);

        if open {
            open_page(&addr);
        }
    })
    .unwrap();
}

fn build_websocket(
    content: String,
    styles: String,
) -> ws::WebSocket<impl ws::Factory<Handler = impl ws::Handler>> {
    ws::Builder::new()
        .build(move |out: ws::Sender| {
            let cloned_content = content.clone();
            let cloned_styles = styles.clone();

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
        .unwrap()
}

fn render_web_template() -> String {
    let html = liquid::ParserBuilder::with_liquid()
        .build()
        .unwrap()
        .parse(WEB_TEMPLATE)
        .unwrap();

    let mut template_values = liquid::value::Object::new();
    template_values.insert("websocketPort".into(), liquid::value::Value::scalar(3012));
    html.render(&template_values).unwrap()
}

fn open_page(addr: &std::net::SocketAddr) {
    std::process::Command::new("open")
        .arg(format!("http://{}", addr))
        .spawn()
        .unwrap();
}

fn watch_and_parse(config: &Cli, output: Sender) {
    let (sender, receiver) = std::sync::mpsc::channel();

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
