extern crate clap;
extern crate termion;
extern crate termios as term;
extern crate unicode_width;

use clap::{App, AppSettings, Arg, ArgMatches};
use unicode_width::UnicodeWidthChar;

use std::fs::File;
use std::io::{self, BufRead, Read, Stdin, Write};
use std::os::unix::io::AsRawFd;
use std::process::{exit, Command};

struct TermDim {
    height: usize,
    width: usize,
}

impl TermDim {
    fn new() -> TermDim {
        let (width, height) = termion::terminal_size().unwrap();

        TermDim {
            width: width as usize,
            height: height as usize,
        }
    }

    fn civis(&self) {
        print!("\x1b[?25l");
    }

    fn cnorm(&self) {
        print!("\x1b[?25h");
    }

    fn clear(&self) {
        for _ in 0..(self.height + 2) {
            println!("\x1b[K");
        }
        print!("\x1b[{}A", self.height + 2);
    }
}

struct ViList {
    list: Vec<String>,
    len: usize,
    selected: usize,
    height: usize,
    width: usize,
}

fn write_line<'t>(stdout: &mut io::StdoutLock, color: &'t str, line: &'t str) {
    stdout
        .write_fmt(format_args!(
            "{}{}\x1b[0m\x1b[K\x1b[1B\x1b[{}D",
            color,
            line,
            line.len()
        ))
        .unwrap();
}

impl ViList {
    fn build(stdin: Stdin, dim: &TermDim) -> ViList {
        let list: Vec<String> = stdin.lock().lines().map(|l| l.unwrap()).collect();
        let len = list.len();

        let height = if dim.height / 2 > len {
            len
        } else {
            dim.height / 2
        };

        ViList {
            height,
            len,
            list,
            selected: 0,
            width: dim.width,
        }
    }

    fn trim_list(&self) -> Vec<String> {
        self.list
            .iter()
            .map(|l| trim_string(l.to_string(), self.width))
            .collect::<Vec<String>>()
    }

    fn start_point(&self) -> (usize, usize) {
        let end = if self.len > self.height {
            let buffer = self.height / 2;

            if self.selected + buffer >= self.len {
                self.len
            } else if self.selected + buffer > self.height {
                self.selected + 1 + buffer
            } else {
                self.height + 1
            }
        } else {
            self.len
        };

        let start = if end < (self.height + 1) {
            0
        } else {
            end - (self.height + 1)
        };

        (start, end)
    }

    fn pct_str(&self) -> String {
        format!(
            "{:3}/{:3}, {:3}%",
            self.selected + 1,
            self.len,
            ((self.selected + 1) * 100) / self.len
        )
    }

    fn display(&self, stdout: &mut io::StdoutLock) {
        let list = self.trim_list();

        let (start, end) = self.start_point();

        let mut drew = start;

        for line in list[start..end].iter() {
            let color = if drew == self.selected {
                "\x1b[1m\x1b[34m"
            } else {
                "\x1b[0m"
            };

            write_line(stdout, color, line);

            drew += 1;
        }

        write_line(stdout, "0", &self.pct_str());

        stdout
            .write_fmt(format_args!("\x1b[{}A", (end - start) + 1))
            .unwrap();

        stdout.flush().unwrap();
    }

    fn selected(&self) -> String {
        self.list[self.selected].to_string()
    }
}

fn trim_string(string: String, tgt: usize) -> String {
    let mut w = 0;

    let mut result = "".to_string();

    for c in string.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1);

        if w + cw > tgt {
            break;
        };

        w += cw;
        result.push(c);
    }

    result
}

fn uncook_tty(fd: i32) -> term::Termios {
    let mut termios = term::Termios::from_fd(fd).unwrap();
    let old_termios = termios;
    term::cfmakeraw(&mut termios);
    term::tcsetattr(fd, term::TCSANOW, &termios).unwrap();

    old_termios
}

fn select_loop(tty: &mut File, list: &mut ViList) -> bool {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut buf = [0; 1];

    loop {
        list.display(&mut writer);

        tty.read_exact(&mut buf[..]).unwrap();

        match buf[0] {
            b'q' => {
                return false;
            }
            b'k' | b'A' | b'h' | b'C' => {
                list.selected = if list.selected > 0 {
                    list.selected - 1
                } else {
                    list.len - 1
                };
            }
            b'j' | b'B' | b'l' | b'D' => {
                list.selected = if list.selected < list.len - 1 {
                    list.selected + 1
                } else {
                    0
                };
            }
            b'g' => {
                list.selected = 0;
            }
            b'G' => {
                list.selected = list.len - 1;
            }
            b'z' => {
                list.selected = list.len / 2;
            }
            13 => {
                break;
            }
            _ => {}
        }
    }

    true
}

fn parse_options() -> ArgMatches<'static> {
    App::new("Visual SELect")
        .version("0.1.0")
        .author("Stone Tickle")
        .about("select a line from stdin and execute the specified command")
        .setting(AppSettings::TrailingVarArg)
        .arg(Arg::with_name("command").required(true).multiple(true))
        .arg(
            Arg::with_name("multi")
                .short("m")
                .help("enables multiple selections"),
        )
        .get_matches()
}

struct Cmd {
    path: String,
    args: Vec<String>,
}

impl Cmd {
    fn parse(parts: clap::Values) -> Cmd {
        let parts: Vec<String> = parts.map(|v| v.to_string()).collect();

        let (head, args) = parts.split_at(1);
        let path = head.first().unwrap().to_string();
        let args = args.iter().map(|v| v.to_string()).collect();

        Cmd { path, args }
    }

    fn exec(&self, value: &str) -> Option<i32> {
        Command::new(&self.path)
            .args(&self.args)
            .arg(value)
            .status()
            .unwrap()
            .code()
    }
}

fn main() {
    let opts = parse_options();
    let cmd = Cmd::parse(opts.values_of("command").unwrap());

    let win = TermDim::new();
    let mut list = ViList::build(io::stdin(), &win);

    if list.len == 0 {
        exit(1);
    };

    let mut stdin = File::open("/dev/tty").unwrap();
    win.clear();
    win.civis();
    let cooked = uncook_tty(stdin.as_raw_fd());

    loop {
        if select_loop(&mut stdin, &mut list) {
            match cmd.exec(&list.selected()) {
                None => exit(1),
                Some(code) => {
                    if code != 0 {
                        exit(code);
                    }
                }
            };

            if !opts.is_present("multi") {
                break;
            }
        } else {
            break;
        }
    }

    term::tcsetattr(stdin.as_raw_fd(), term::TCSANOW, &cooked).unwrap();
    win.clear();
    win.cnorm();
    print!("\x1b[1A\x1b[K");
}
