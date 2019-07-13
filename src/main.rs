extern crate clap;
extern crate termion;
extern crate termios as term;
extern crate unicode_width;

use clap::{App, AppSettings, Arg, ArgMatches};
use unicode_width::UnicodeWidthChar;

use std::fs::File;
use std::io::{self, BufRead, Read, Stdin, Write};
use std::os::unix::io::AsRawFd;
use std::process::Command;

fn trim_string(string: String, tgt: usize) -> String {
    let mut w = 0;

    let mut result = "".to_string();

    for c in string.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1);

        if w + cw > tgt {
            break;
        };

        w = w + cw;
        result.push(c);
    }

    result
}

fn starting_point(sel: usize, height: usize, length: usize) -> (usize, usize) {
    let end = if length > height {
        let buffer = height / 2;

        if sel + buffer >= length {
            length
        } else if sel + buffer > height {
            sel + 1 + buffer
        } else {
            height + 1
        }
    } else {
        length
    };

    let start = if end < (height + 1) {
        0
    } else {
        end - (height + 1)
    };

    (start, end)
}

fn display_list(
    list: &Vec<String>,
    selected: usize,
    height: usize,
    width: usize,
    stdout: &mut io::StdoutLock,
) {
    let my_list = list
        .into_iter()
        .map(|l| trim_string(l.to_string(), width))
        .collect::<Vec<String>>();
    let list = &my_list;

    let (start, end) = starting_point(selected, height, list.len());

    let mut drew = start;

    for s in list[start..end].iter() {
        //println!("{:?} ", s);
        let color = if drew == selected {
            "\x1b[1m\x1b[34m"
        } else {
            "\x1b[0m"
        };

        let line_length = s.len();

        stdout
            .write_fmt(format_args!(
                "{}{}\x1b[0m\x1b[K\x1b[1B\x1b[{}D",
                color, s, line_length
            ))
            .unwrap();

        drew = drew + 1;
    }

    let s = format!(
        "{:3}/{:3}, {:3}%",
        selected + 1,
        list.len(),
        ((selected + 1) * 100) / list.len()
    );

    let line_length = s.len();

    stdout
        .write_fmt(format_args!(
            "{}\x1b[0m\x1b[K\x1b[1B\x1b[{}D",
            s, line_length
        ))
        .unwrap();

    stdout
        .write_fmt(format_args!("\x1b[{}A", (drew - start) + 1))
        .unwrap();
    stdout.flush().unwrap();
}

fn grab_stdin(stdin: Stdin) -> Vec<String> {
    stdin.lock().lines().map(|l| l.unwrap()).collect()
}

fn uncook_tty(fd: i32) -> term::Termios {
    let mut termios = term::Termios::from_fd(fd).unwrap();
    let old_termios = termios.clone();
    term::cfmakeraw(&mut termios);
    term::tcsetattr(fd, term::TCSANOW, &termios).unwrap();

    old_termios
}

fn clear_display(len: usize) {
    for _ in 0..(len + 2) {
        println!("\x1b[K");
    }
    print!("\x1b[{}A", len + 2);
}

fn select_loop(
    tty: &mut File,
    start: usize,
    height: usize,
    width: usize,
    lines: &Vec<String>,
) -> (Option<String>, Option<usize>) {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut buf = [0; 1];

    let list_length = lines.len();
    let mut selected: usize = start;

    loop {
        display_list(lines, selected, height, width, &mut writer);
        writer.flush().unwrap();

        tty.read_exact(&mut buf[..]).unwrap();

        match buf[0] {
            b'q' => {
                return (None, None);
            }
            b'k' | b'A' | b'h' | b'C' => {
                selected = if selected > 0 {
                    selected - 1
                } else {
                    list_length - 1
                };
            }
            b'j' | b'B' | b'l' | b'D' => {
                selected = if selected < list_length - 1 {
                    selected + 1
                } else {
                    0
                };
            }
            b'g' => {
                selected = 0;
            }
            b'G' => {
                selected = list_length - 1;
            }
            b'z' => {
                selected = list_length / 2;
            }
            13 => {
                break;
            }
            _ => {}
        }
    }

    (Some(lines[selected].to_string()), Some(selected))
}

fn parse_options() -> ArgMatches<'static> {
    App::new("Visual SELect")
        .version("0.1.0")
        .author("Stone Tickle")
        .about("select a line from stdin and execute the specified command")
        .setting(AppSettings::TrailingVarArg)
        .arg(Arg::with_name("command").required(true).multiple(true))
        .get_matches()
}

fn main() -> Result<(), std::io::Error> {
    let opts = parse_options();

    let stdin = io::stdin();

    let (width, height) = termion::terminal_size().unwrap();
    let (width, height) = (width as usize, height as usize);

    let lines = grab_stdin(stdin);

    let height = if height / 2 > lines.len() {
        lines.len()
    } else {
        height / 2
    };

    if lines.len() <= 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty input"));
    };

    let mut tty = File::open("/dev/tty").unwrap();

    let command_parts: Vec<String> = opts
        .values_of("command")
        .unwrap()
        .map(|v| v.to_string())
        .collect();

    let (head, args) = command_parts.split_at(1);
    let cmd_path = head.first().unwrap();

    print!("\x1b[?25l");
    clear_display(height);
    let cooked = uncook_tty(tty.as_raw_fd());

    let mut rresult = Ok(());
    let mut result_ok = true;
    let mut start = 0;
    while result_ok {
        let (selection, id) = select_loop(&mut tty, start, height, width, &lines);
        rresult = match selection {
            Some(val) => {
                match Command::new(cmd_path)
                    .args(args)
                    .arg(val)
                    .status()
                    .unwrap()
                    .code()
                {
                    None => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "command killed by signal",
                    )), // the command was killed by a signal,
                    Some(code) => match code {
                        0 => Ok(()),
                        _ => Err(io::Error::from_raw_os_error(code)),
                    },
                }
            }
            None => Err(io::Error::new(io::ErrorKind::Other, "nothing selected")),
        };

        match rresult {
            Ok(_) => {
                start = id.unwrap();
            }
            Err(_) => {
                result_ok = false;
            }
        }
    }

    term::tcsetattr(tty.as_raw_fd(), term::TCSANOW, &cooked).unwrap();
    clear_display(height);

    print!("\x1b[?25h\x1b[1A\x1b[K");
    return rresult;
}
