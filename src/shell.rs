// This file was adapted from:
//   https://github.com/rust-lang/cargo/blob/ca4edabb28fc96fdf2a1d56fe3851831ac166f8a/src/cargo/core/shell.rs

use std::fmt;
use std::io::{self, Write};

use crate::errors::Result;
use owo_colors::{self, OwoColorize};

// get the prefix for stderr messages
macro_rules! cross_prefix {
    ($s:literal) => {
        concat!("[cross]", " ", $s)
    };
}

// generate the color style
macro_rules! write_style {
    ($stream:ident, $msg_info:expr, $message:expr $(, $style:ident)* $(,)?) => {{
        match $msg_info.color_choice {
            ColorChoice::Always => write!($stream, "{}", $message $(.$style())*),
            ColorChoice::Never => write!($stream, "{}", $message),
            ColorChoice::Auto => write!(
                $stream,
                "{}",
                $message $(.if_supports_color($stream.owo(), |text| text.$style()))*
            ),
        }?;
    }};
}

// low-level interface for printing colorized messages
macro_rules! message {
    // write a status message, which has the following format:
    //  "{status}: {message}"
    // both status and ':' are bold.
    (@status $stream:ident, $status:expr, $message:expr, $color:ident, $msg_info:expr $(,)?) => {{
        write_style!($stream, $msg_info, $status, bold, $color);
        write_style!($stream, $msg_info, ":", bold);
        match $message {
            Some(message) => writeln!($stream, " {}", message)?,
            None => write!($stream, " ")?,
        }

        Ok(())
    }};

    (@status @name $name:ident, $status:expr, $message:expr, $color:ident, $msg_info:expr $(,)?) => {{
        let mut stream = io::$name();
        message!(@status stream, $status, $message, $color, $msg_info)
    }};
}

// high-level interface to message
macro_rules! status {
    (@stderr $status:expr, $message:expr, $color:ident, $msg_info:expr $(,)?) => {{
        message!(@status @name stderr, $status, $message, $color, $msg_info)
    }};

    (@stdout $status:expr, $message:expr, $color:ident, $msg_info:expr  $(,)?) => {{
        message!(@status @name stdout, $status, $message, $color, $msg_info)
    }};
}

/// the requested verbosity of output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

impl Verbosity {
    pub fn verbose(self) -> bool {
        match self {
            Self::Verbose => true,
            Self::Normal | Self::Quiet => false,
        }
    }
}

/// Whether messages should use color output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    /// force color output
    Always,
    /// force disable color output
    Never,
    /// intelligently guess whether to use color output
    Auto,
}

// Should simplify the APIs a lot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageInfo {
    pub color_choice: ColorChoice,
    pub verbosity: Verbosity,
    pub stdout_needs_erase: bool,
    pub stderr_needs_erase: bool,
}

impl MessageInfo {
    pub const fn new(color_choice: ColorChoice, verbosity: Verbosity) -> MessageInfo {
        MessageInfo {
            color_choice,
            verbosity,
            stdout_needs_erase: false,
            stderr_needs_erase: false,
        }
    }

    pub fn create(verbose: bool, quiet: bool, color: Option<&str>) -> Result<MessageInfo> {
        let color_choice = get_color_choice(color)?;
        let verbosity = get_verbosity(color_choice, verbose, quiet)?;

        Ok(MessageInfo {
            color_choice,
            verbosity,
            stdout_needs_erase: false,
            stderr_needs_erase: false,
        })
    }

    pub fn is_verbose(&self) -> bool {
        self.verbosity.verbose()
    }

    fn as_verbosity<T, C: Fn(&mut MessageInfo) -> T>(&mut self, call: C, new: Verbosity) -> T {
        let old = self.verbosity;
        self.verbosity = new;
        let result = call(self);
        self.verbosity = old;

        result
    }

    pub fn as_quiet<T, C: Fn(&mut MessageInfo) -> T>(&mut self, call: C) -> T {
        self.as_verbosity(call, Verbosity::Quiet)
    }

    pub fn as_normal<T, C: Fn(&mut MessageInfo) -> T>(&mut self, call: C) -> T {
        self.as_verbosity(call, Verbosity::Normal)
    }

    pub fn as_verbose<T, C: Fn(&mut MessageInfo) -> T>(&mut self, call: C) -> T {
        self.as_verbosity(call, Verbosity::Verbose)
    }

    fn erase_line<S: Stream + Write>(&mut self, stream: &mut S) -> Result<()> {
        // this is the Erase in Line sequence
        stream.write_all(b"\x1B[K").map_err(Into::into)
    }

    fn stdout_check_erase(&mut self) -> Result<()> {
        if self.stdout_needs_erase {
            self.erase_line(&mut io::stdout())?;
            self.stdout_needs_erase = false;
        }
        Ok(())
    }

    fn stderr_check_erase(&mut self) -> Result<()> {
        if self.stderr_needs_erase {
            self.erase_line(&mut io::stderr())?;
            self.stderr_needs_erase = false;
        }
        Ok(())
    }

    /// prints a red 'error' message and terminates.
    pub fn fatal<T: fmt::Display>(&mut self, message: T, code: i32) -> ! {
        self.error(message).unwrap();
        std::process::exit(code);
    }

    /// prints a red 'error' message.
    pub fn error<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        self.stderr_check_erase()?;
        status!(@stderr cross_prefix!("error"), Some(&message), red, self)
    }

    /// prints an amber 'warning' message.
    pub fn warn<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => status!(@stderr
                cross_prefix!("warning"),
                Some(&message),
                yellow,
                self,
            ),
        }
    }

    /// prints a cyan 'note' message.
    pub fn note<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => status!(@stderr cross_prefix!("note"), Some(&message), cyan, self),
        }
    }

    pub fn status<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => {
                eprintln!("{}", message);
                Ok(())
            }
        }
    }

    /// prints a high-priority message to stdout.
    pub fn print<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        self.stdout_check_erase()?;
        println!("{}", message);
        Ok(())
    }

    /// prints a normal message to stdout.
    pub fn info<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => {
                println!("{}", message);
                Ok(())
            }
        }
    }

    /// prints a debugging message to stdout.
    pub fn debug<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet | Verbosity::Normal => Ok(()),
            _ => {
                println!("{}", message);
                Ok(())
            }
        }
    }

    pub fn fatal_usage<T: fmt::Display>(&mut self, arg: T, code: i32) -> ! {
        self.error_usage(arg).unwrap();
        std::process::exit(code);
    }

    fn error_usage<T: fmt::Display>(&mut self, arg: T) -> Result<()> {
        let mut stream = io::stderr();
        write_style!(stream, self, cross_prefix!("error"), bold, red);
        write_style!(stream, self, ":", bold);
        write_style!(stream, self, " The argument '");
        write_style!(stream, self, arg, yellow);
        write_style!(stream, self, "' requires a value but none was supplied\n");
        write_style!(stream, self, "Usage:\n");
        write_style!(
            stream,
            self,
            "    cross [+toolchain] [OPTIONS] [SUBCOMMAND]\n"
        );
        write_style!(stream, self, "\n");
        write_style!(stream, self, "For more information try ");
        write_style!(stream, self, "--help", green);
        write_style!(stream, self, "\n");

        stream.flush()?;

        Ok(())
    }
}

impl Default for MessageInfo {
    fn default() -> MessageInfo {
        MessageInfo::new(ColorChoice::Auto, Verbosity::Normal)
    }
}

impl From<ColorChoice> for MessageInfo {
    fn from(color_choice: ColorChoice) -> MessageInfo {
        MessageInfo::new(color_choice, Verbosity::Normal)
    }
}

impl From<Verbosity> for MessageInfo {
    fn from(verbosity: Verbosity) -> MessageInfo {
        MessageInfo::new(ColorChoice::Auto, verbosity)
    }
}

impl From<(ColorChoice, Verbosity)> for MessageInfo {
    fn from(info: (ColorChoice, Verbosity)) -> MessageInfo {
        MessageInfo::new(info.0, info.1)
    }
}

fn get_color_choice(color: Option<&str>) -> Result<ColorChoice> {
    match color {
        Some("always") => Ok(ColorChoice::Always),
        Some("never") => Ok(ColorChoice::Never),
        Some("auto") | None => Ok(ColorChoice::Auto),
        Some(arg) => {
            eyre::bail!("argument for --color must be auto, always, or never, but found `{arg}`")
        }
    }
}

fn get_verbosity(color_choice: ColorChoice, verbose: bool, quiet: bool) -> Result<Verbosity> {
    match (verbose, quiet) {
        (true, true) => {
            MessageInfo::from(color_choice).error("cannot set both --verbose and --quiet")?;
            std::process::exit(101);
        }
        (true, false) => Ok(Verbosity::Verbose),
        (false, true) => Ok(Verbosity::Quiet),
        (false, false) => Ok(Verbosity::Normal),
    }
}

pub trait Stream {
    const TTY: atty::Stream;
    const OWO: owo_colors::Stream;

    fn is_atty() -> bool {
        atty::is(Self::TTY)
    }

    fn owo(&self) -> owo_colors::Stream {
        Self::OWO
    }
}

impl Stream for io::Stdin {
    const TTY: atty::Stream = atty::Stream::Stdin;
    const OWO: owo_colors::Stream = owo_colors::Stream::Stdin;
}

impl Stream for io::Stdout {
    const TTY: atty::Stream = atty::Stream::Stdout;
    const OWO: owo_colors::Stream = owo_colors::Stream::Stdout;
}

impl Stream for io::Stderr {
    const TTY: atty::Stream = atty::Stream::Stderr;
    const OWO: owo_colors::Stream = owo_colors::Stream::Stderr;
}

pub fn default_ident() -> usize {
    cross_prefix!("").len()
}

pub fn indent(message: &str, spaces: usize) -> String {
    message
        .lines()
        .map(|s| format!("{:spaces$}{s}", ""))
        .collect()
}
