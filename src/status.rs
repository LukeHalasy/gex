//! Module relating to the Status display, including diffs of files.

use std::{
    fmt, fs,
    io::{stdout, Write},
    process::{Command, Stdio},
};

use crossterm::style::{self, Attribute, Color};
use nom::{
    bytes::complete::{tag, take_until},
    IResult,
};

use crate::{git_process, parse};

pub trait Expand {
    fn toggle_expand(&mut self);
    fn expanded(&self) -> bool;
}

#[derive(Debug)]
enum DiffType {
    Modified,
    Created,
    Untracked,
    Renamed,
    Deleted,
}

#[derive(Debug)]
struct Hunk {
    diffs: Vec<String>,
    expanded: bool,
}

impl fmt::Display for Hunk {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use fmt::Write;
        let mut outbuf = format!(
            "\r\n{}{}{}",
            style::SetForegroundColor(Color::Blue),
            match self.expanded {
                true => "⌄",
                false => "›",
            },
            self.diffs[0].replace(" @@", &format!(" @@{}", Attribute::Reset))
        );

        if self.expanded {
            for line in self.diffs.iter().skip(1) {
                write!(
                    &mut outbuf,
                    "\r\n{}{}",
                    match line.chars().next() {
                        Some('+') => style::SetForegroundColor(Color::DarkGreen),
                        Some('-') => style::SetForegroundColor(Color::DarkRed),
                        _ => style::SetForegroundColor(Color::Reset),
                    },
                    line
                )?;
            }
        }
        write!(f, "{}", outbuf)
    }
}

impl Hunk {
    fn new(diffs: Vec<String>) -> Self {
        Self {
            diffs,
            expanded: false,
        }
    }
}

impl Expand for Hunk {
    fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
    }

    fn expanded(&self) -> bool {
        self.expanded
    }
}

#[derive(Debug)]
pub struct FileDiff {
    path: String,
    expanded: bool,
    diff: Vec<Hunk>,
    cursor: usize,
    kind: DiffType,
}

impl fmt::Display for FileDiff {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "\r{}{}{}",
            match self.expanded {
                true => "⌄",
                false => "›",
            },
            match self.kind {
                DiffType::Renamed => "[RENAME] ",
                DiffType::Deleted => "[DELETE] ",
                _ => "",
            },
            self.path,
        )?;
        if self.expanded {
            match self.diff.is_empty() {
                true => {
                    if let Ok(file_content) = fs::read_to_string(&self.path) {
                        let file_content: String =
                            file_content.lines().collect::<Vec<&str>>().join("\r\n+");

                        write!(
                            f,
                            "\r\n{}{}+{}",
                            Attribute::Reset,
                            style::SetForegroundColor(Color::DarkGreen),
                            file_content
                        )?;
                    }
                }
                false => {
                    for (i, hunk) in self.diff.iter().enumerate() {
                        if i + 1 == self.cursor {
                            write!(f, "{}{}{}", Attribute::Reset, Attribute::Reverse, hunk)?;
                        } else {
                            write!(f, "{}{}", Attribute::Reset, hunk)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl FileDiff {
    fn new(path: &str, kind: DiffType) -> Self {
        Self {
            path: path.to_string(),
            expanded: false,
            diff: Vec::new(),
            cursor: 0,
            kind,
        }
    }

    /// Fails on the case that we are already on the first hunk
    fn up(&mut self) -> Result<(), ()> {
        match self.cursor.checked_sub(1) {
            Some(val) => {
                self.cursor = val;
                Ok(())
            }
            None => Err(()),
        }
    }

    /// Fails on the case that we are already on the final hunk
    fn down(&mut self) -> Result<(), ()> {
        self.cursor += 1;
        if self.cursor >= self.len() {
            return Err(());
        }

        Ok(())
    }

    fn len(&self) -> usize {
        match self.expanded {
            true => self.diff.len() + 1,
            false => 1,
        }
    }
}

impl Expand for FileDiff {
    fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
    }

    fn expanded(&self) -> bool {
        self.expanded
    }
}

// Enum for `Status.stage_or_unstage`
#[derive(Clone, Copy)]
enum Stage {
    Add,
    Reset,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "{}",
            match self {
                Stage::Add => "add",
                Stage::Reset => "reset",
            }
        )
    }
}

#[derive(Debug, Default)]
pub struct Status {
    pub branch: String,
    pub head: String,
    pub diffs: Vec<FileDiff>,
    pub count_untracked: usize,
    pub count_unstaged: usize,
    pub count_staged: usize,
    pub cursor: usize,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        // Display the current branch
        writeln!(
            f,
            "\rOn branch {}{}{}",
            Attribute::Bold,
            self.branch,
            Attribute::Reset
        )?;

        // Display most recent commit
        if !self.head.is_empty() {
            let mut head = self.head.split_whitespace();
            writeln!(
                f,
                "{}\r\n{}{}{}",
                Attribute::Dim,
                head.next().unwrap(),
                Attribute::Reset,
                head.map(|w| format!(" {}", w)).collect::<String>()
            )?;
        }

        if self.diffs.is_empty() {
            write!(
                f,
                "\r\n{}nothing to commit, working tree clean{}",
                style::SetForegroundColor(Color::Yellow),
                style::SetForegroundColor(Color::Reset)
            )?;
            let _ = stdout().flush();
        }

        for (index, file) in self.diffs.iter().enumerate() {
            if index == 0 && self.count_untracked != 0 {
                write!(
                    f,
                    "\r\n{}Untracked files:{}\n",
                    style::SetForegroundColor(Color::Yellow),
                    style::ResetColor
                )?;
            } else if index == self.count_untracked && self.count_unstaged != 0 {
                write!(
                    f,
                    "\r\n{}Unstaged changes:{}\n",
                    style::SetForegroundColor(Color::Yellow),
                    style::ResetColor
                )?;
            } else if index == self.count_untracked + self.count_unstaged {
                write!(
                    f,
                    "\r\n{}Staged changes:{}\n",
                    style::SetForegroundColor(Color::Yellow),
                    style::ResetColor
                )?;
            }

            if file.cursor == 0 && self.cursor == index {
                write!(f, "{}", Attribute::Reverse)?;
            }
            writeln!(f, "\r    {}{}", file, Attribute::Reset)?;
        }

        Ok(())
    }
}

impl Status {
    pub fn new() -> Self {
        let mut status = Self::default();
        status.fetch();
        status
    }

    pub fn fetch(&mut self) {
        let output = git_process(&["status"]);

        let input = std::str::from_utf8(&output.stdout).unwrap();

        let mut lines = input.lines();
        let branch_line = lines.next().expect("not a valid `git status` output");
        let branch: IResult<&str, &str> = tag("On branch ")(branch_line);
        let (branch, _) = branch.unwrap();

        let mut untracked = Vec::new();
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        while let Some(line) = lines.next() {
            if line == "Untracked files:" {
                lines.next().unwrap(); // Skip message from git
                'untrackeds: for line in lines.by_ref() {
                    if line.is_empty() {
                        break 'untrackeds;
                    }
                    untracked.push(FileDiff::new(line.trim_start(), DiffType::Untracked));
                }
            } else if line == "Changes to be committed:" {
                lines.next().unwrap(); // Skip message from git
                'staged: for line in lines.by_ref() {
                    if line.is_empty() {
                        break 'staged;
                    }

                    let parse_result: IResult<&str, &str> = take_until("  ")(line.trim_start());
                    let (line, prefix) = parse_result.expect("strange diff output");

                    staged.push(FileDiff::new(
                        line.trim_start(),
                        match prefix {
                            "" => DiffType::Untracked,        // untracked files
                            "new file:" => DiffType::Created, // staged new files
                            "modified:" => DiffType::Modified,
                            "renamed:" => DiffType::Renamed,
                            "deleted:" => DiffType::Deleted,
                            _ => panic!("Unknown prefix: `{}`", prefix),
                        },
                    ));
                }
            } else if line == "Changes not staged for commit:" {
                lines.next().unwrap(); // Skip message from git
                lines.next().unwrap();
                'unstaged: for line in lines.by_ref() {
                    if line.is_empty() {
                        break 'unstaged;
                    }

                    let parse_result: IResult<&str, &str> = take_until("  ")(line.trim_start());
                    let (line, prefix) = parse_result.expect("strange diff output");

                    unstaged.push(FileDiff::new(
                        line.trim_start(),
                        match prefix {
                            "" => DiffType::Untracked,        // untracked files
                            "new file:" => DiffType::Created, // staged new files
                            "modified:" => DiffType::Modified,
                            "renamed:" => DiffType::Renamed,
                            "deleted:" => DiffType::Deleted,
                            _ => panic!("Unknown prefix: `{}`", prefix),
                        },
                    ));
                }
            }
        }

        let diff = git_process(&["diff"]);
        let staged_diff = git_process(&["diff", "--cached"]);

        let diff = std::str::from_utf8(&diff.stdout).unwrap();
        let diffs = parse::parse_diff(diff);
        'outer_unstaged: for (path, diff) in diffs {
            for mut file in &mut unstaged {
                if file.path == path {
                    file.diff = diff
                        .iter()
                        .map(|d| {
                            Hunk::new(
                                d.to_owned()
                                    .iter()
                                    .map(|l| l.to_string())
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>();
                    continue 'outer_unstaged;
                }
            }
        }

        let staged_diff = std::str::from_utf8(&staged_diff.stdout).unwrap();
        let diffs = parse::parse_diff(staged_diff);
        'outer_staged: for (path, diff) in diffs {
            for mut file in &mut staged {
                if file.path == path {
                    file.diff = diff
                        .iter()
                        .map(|d| {
                            Hunk::new(
                                d.to_owned()
                                    .iter()
                                    .map(|l| l.to_string())
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>();
                    continue 'outer_staged;
                }
            }
        }

        self.branch = branch.to_string();
        self.head = std::str::from_utf8(
            &git_process(&["log", "HEAD", "--pretty=format:\"%h %s\"", "-n", "1"]).stdout,
        )
        .expect("invalid utf8 from `git log`")
        .trim_start_matches('\"')
        .trim_end_matches('\"')
        .to_string();
        self.count_untracked = untracked.len();
        self.count_staged = staged.len();
        self.count_unstaged = unstaged.len();

        self.diffs = untracked;
        self.diffs.append(&mut unstaged);
        self.diffs.append(&mut staged);

        if !self.diffs.is_empty() && self.cursor >= self.diffs.len() {
            self.cursor = self.diffs.len() - 1;
        }
    }

    fn stage_or_unstage(&mut self, command: Stage) {
        if self.diffs.is_empty() {
            return;
        }

        let file = self.diffs.get_mut(self.cursor).unwrap();
        match file.cursor {
            0 => {
                let args = match command {
                    Stage::Add => vec!["add", &file.path],
                    Stage::Reset => match file.kind {
                        DiffType::Deleted => vec!["reset", "HEAD", &file.path],
                        _ => vec!["reset", &file.path],
                    },
                };
                git_process(&args);
            }
            i => {
                let mut patch = Command::new("git")
                    .args(match command {
                        Stage::Add => ["add", "-p", &file.path],
                        Stage::Reset => ["reset", "-p", &file.path],
                    })
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .expect("failed to spawn interactive git process");

                let mut stdin = patch.stdin.take().expect("failed to open child stdin");

                let mut bufs = vec![b"n\n"; i - 1];
                bufs.push(b"y\n");

                std::thread::spawn(move || {
                    for buf in bufs {
                        stdin.write_all(buf).expect("failed to patch hunk");
                    }
                });

                let _ = patch.wait();
            }
        }
        self.fetch();
    }

    pub fn stage(&mut self) {
        self.stage_or_unstage(Stage::Add);
    }

    pub fn unstage(&mut self) {
        self.stage_or_unstage(Stage::Reset);
    }

    /// Toggles expand on the selected diff item.
    pub fn expand(&mut self) {
        if self.diffs.is_empty() {
            return;
        }

        let mut file = self.diffs.get_mut(self.cursor).unwrap();
        if file.cursor == 0 {
            file.expanded = !file.expanded;
        } else {
            file.diff[file.cursor - 1].expanded = !file.diff[file.cursor - 1].expanded;
        }
    }

    /// Move the cursor up one
    pub fn up(&mut self) {
        if self.diffs.is_empty() {
            return;
        }

        let file = self.diffs.get_mut(self.cursor).unwrap();
        if file.up().is_err() {
            match self.cursor.checked_sub(1) {
                Some(v) => {
                    self.cursor = v;
                    let _ = self.diffs[self.cursor].up();
                }
                None => self.cursor = 0,
            }
        }
    }

    /// Move the cursor down one
    pub fn down(&mut self) {
        if self.diffs.is_empty() {
            return;
        }

        let file = self.diffs.get_mut(self.cursor).unwrap();
        if file.down().is_err() {
            self.cursor += 1;
            if self.cursor >= self.diffs.len() {
                self.cursor = self.diffs.len() - 1;
                self.up();
            }
        }
    }
}
