#![doc = include_str!("../README.md")]

mod addr;
mod colors;
mod event;
mod mapi;
mod pcap;
mod proxy;
mod render;

use std::fs::File;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;
use std::{io, panic, process, thread};

use addr::MonetAddr;
use anyhow::{bail, Context, Result as AResult};
use argsplitter::{ArgError, ArgSplitter};
use colors::{NO_COLORS, VT100_COLORS};
use event::{MapiEvent, Timestamp};
use pcap::Tracker;

use crate::{proxy::Proxy, render::Renderer};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const USAGE: &str = include_str!("usage.txt");

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum Level {
    Raw,
    Blocks,
    Messages,
}

#[derive(Debug)]
enum Source {
    Proxy {
        listen_addr: MonetAddr,
        forward_addr: MonetAddr,
    },
    Pcap(PathBuf),
}

fn main() -> ExitCode {
    argsplitter::main_support::report_errors(USAGE, mymain())
}

fn mymain() -> AResult<()> {
    install_panic_hook();

    let mut output_file: Option<PathBuf> = None;
    let mut pcap_file: Option<PathBuf> = None;
    let mut level = None;
    let mut force_binary = false;
    let mut colors = None;

    let mut args = ArgSplitter::from_env();
    while let Some(flag) = args.flag()? {
        match flag {
            "-o" | "--output" => output_file = Some(args.param_os()?.into()),
            "--pcap" => pcap_file = Some(args.param_os()?.into()),
            "-m" | "--messages" => level = Some(Level::Messages),
            "-b" | "--blocks" => level = Some(Level::Blocks),
            "-r" | "--raw" => level = Some(Level::Raw),
            "-B" | "--binary" => force_binary = true,
            "--color" => {
                colors = match args.param()?.to_lowercase().as_str() {
                    "always" => Some(VT100_COLORS),
                    "auto" => None,
                    "never" => Some(NO_COLORS),
                    other => bail!("--color={other}: must be 'always', 'auto' or 'never'"),
                }
            }
            "--help" => {
                println!("Mapiproxy version {VERSION}");
                println!();
                println!("{USAGE}");
                return Ok(());
            }
            "--version" => {
                println!("Mapiproxy version {VERSION}");
                return Ok(());
            }
            _ => return Err(ArgError::unknown_flag(flag).into()),
        }
    }
    let Some(level) = level else {
        return Err(ArgError::message("Please set the mode using -r, -b or -m").into());
    };

    let source = if let Some(path) = pcap_file {
        Source::Pcap(path)
    } else {
        let listen_addr = args.stashed_os("LISTEN_ADDR")?.try_into()?;
        let forward_addr = args.stashed_os("FORWARD_ADDR")?.try_into()?;
        Source::Proxy {
            listen_addr,
            forward_addr,
        }
    };

    args.no_more_stashed()?;

    let default_colors;
    let out: Box<dyn io::Write + Send + 'static> = if let Some(p) = output_file {
        let f = File::create(&p)
            .with_context(|| format!("could not open output file {}", p.display()))?;
        default_colors = NO_COLORS;
        Box::new(f)
    } else {
        let out = io::stdout();
        default_colors = if is_terminal::is_terminal(&out) {
            VT100_COLORS
        } else {
            NO_COLORS
        };
        Box::new(out)
    };
    let mut renderer = Renderer::new(colors.unwrap_or(default_colors), out);

    let mapi_state = mapi::State::new(level, force_binary);

    match source {
        Source::Proxy {
            listen_addr,
            forward_addr,
        } => run_proxy(listen_addr, forward_addr, mapi_state, &mut renderer),
        Source::Pcap(path) => run_pcap(&path, mapi_state, &mut renderer),
    }
}

fn run_proxy(
    listen_addr: MonetAddr,
    forward_addr: MonetAddr,
    mut mapi_state: mapi::State,
    renderer: &mut Renderer,
) -> AResult<()> {
    let (send_events, receive_events) = std::sync::mpsc::sync_channel(500);
    let handler = move |event| {
        let timestamp = SystemTime::now().into();
        let _ = send_events.send((timestamp, event));
    };
    let mut proxy = Proxy::new(listen_addr, forward_addr, handler)?;
    install_ctrl_c_handler(proxy.get_shutdown_trigger())?;
    thread::spawn(move || proxy.run().unwrap());

    while let Ok((ts, ev)) = receive_events.recv() {
        mapi_state.handle(&ts, &ev, renderer)?;
    }
    Ok(())
}

fn run_pcap(path: &Path, mut mapi_state: mapi::State, renderer: &mut Renderer) -> AResult<()> {
    let mut owned_file;
    let mut owned_stdin;

    let reader: &mut dyn io::Read = if path == Path::new("-") {
        owned_stdin = Some(io::stdin().lock());
        owned_stdin.as_mut().unwrap()
    } else {
        let file = File::open(path)
            .with_context(|| format!("Could not open pcap file {}", path.display()))?;
        owned_file = Some(file);
        owned_file.as_mut().unwrap()
    };

    let handler = |ts: &Timestamp, ev: MapiEvent| mapi_state.handle(ts, &ev, renderer);
    let mut tracker = Tracker::new(handler);
    pcap::parse_pcap_file(reader, &mut tracker)
}

fn install_ctrl_c_handler(trigger: Box<dyn Fn() + Send + Sync>) -> AResult<()> {
    let mut triggered = false;
    let handler = move || {
        if triggered {
            std::process::exit(1);
        }
        triggered = true;
        trigger()
    };
    ctrlc::set_handler(handler).with_context(|| "cannot set Ctrl-C handler")?;
    Ok(())
}

fn install_panic_hook() {
    let orig_hook = panic::take_hook();
    let my_hook = Box::new(move |panic_info: &PanicHookInfo<'_>| {
        orig_hook(panic_info);
        process::exit(1);
    });
    panic::set_hook(my_hook);
}
