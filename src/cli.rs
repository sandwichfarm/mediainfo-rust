//! Command line front end, argument-compatible with the reference `mediainfo` tool for the common
//! options.

use crate::{MediaInfo, Output};
use std::path::{Path, PathBuf};

const HELP: &str = "Usage: mediainfo [-Options...] FileName1 [Filename2...]

Options:
  --Help, -h            Display this help and exit
  --Version             Display version and exit
  --Full, -f            Full information (all fields)
  --Output=FORMAT       Text (default), XML, OLDXML, JSON
  --Inform=TEMPLATE     Custom template: General;%FileName% %Duration/String%
  --Language=raw        Use raw field names as labels
  --Info-Parameters     List every parameter name
  --Recursive, -R       Descend into directories
  --LogFile=FILE        Also write the report to FILE
  --Quiet               No output (exit code only)
";

pub fn run(args: Vec<String>) -> i32 {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut mi = MediaInfo::new();
    let mut recursive = false;
    let mut quiet = false;
    let mut log_file: Option<PathBuf> = None;
    for a in &args {
        let (name, value) = match a.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (a.as_str(), None),
        };
        let lname = name.to_ascii_lowercase();
        match lname.as_str() {
            "--help" | "-h" | "--h" | "-help" => {
                print!("{HELP}");
                return 0;
            }
            "--version" => {
                println!("mediainfo-rust v{}", crate::VERSION);
                println!("{}", mi.option("Info_Version", ""));
                return 0;
            }
            "--full" | "-f" | "--complete" => {
                mi.option("Complete", "1");
            }
            "--output" => {
                mi.option("Output", value.unwrap_or(""));
            }
            "--inform" => {
                mi.option("Inform", value.unwrap_or(""));
            }
            "--language" => {
                mi.option("Language", value.unwrap_or(""));
            }
            "--info-parameters" | "--info_parameters" => {
                print!("{}", mi.option("Info_Parameters", ""));
                return 0;
            }
            "--recursive" | "-r" => recursive = true,
            "--quiet" | "-q" => quiet = true,
            "--logfile" => log_file = value.map(PathBuf::from),
            _ if name.starts_with("--") => {
                // Generic option pass-through (e.g. --ParseSpeed=1).
                let r = mi.option(&name[2..], value.unwrap_or(""));
                if r == "Option not known" {
                    eprintln!("Unknown option: {a}");
                    return 2;
                }
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        print!("{HELP}");
        return 1;
    }
    let mut expanded: Vec<PathBuf> = Vec::new();
    for f in files {
        if f.is_dir() {
            collect(&f, recursive, &mut expanded);
        } else {
            expanded.push(f);
        }
    }
    expanded.sort();
    let mut all = String::new();
    let mut status = 0;
    let multi = expanded.len() > 1;
    for (i, f) in expanded.iter().enumerate() {
        if !f.exists() {
            eprintln!("{}: file not found", f.display());
            status = 1;
            continue;
        }
        mi.open(f);
        let mut report = mi.inform();
        if multi && matches!(mi.options.output, Output::Text) && i + 1 < expanded.len() && mi.options.template.is_none() {
            report.push('\n');
        }
        all.push_str(&report);
        mi.close();
    }
    if !quiet {
        print!("{all}");
    }
    if let Some(p) = log_file {
        if let Err(e) = std::fs::write(&p, &all) {
            eprintln!("{}: {e}", p.display());
            status = 1;
        }
    }
    status
}

fn collect(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recursive {
                collect(&p, recursive, out);
            }
        } else {
            out.push(p);
        }
    }
}
