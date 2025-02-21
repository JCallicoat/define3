extern crate colored;
extern crate define3;
extern crate getopts;
extern crate nom;
extern crate regex;
extern crate rusqlite;
extern crate textwrap;

use define3::Meaning;

use colored::*;
use getopts::Options;
use regex::{Captures, Regex};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::process::exit;

// word -> lang -> poses -> defns
type WordMap = BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<String>>>>;

fn get_defns_by_lang(conn: &Connection, word: &str, search_partial: bool) -> Box<WordMap> {
    let mut stmt = if search_partial {
        conn.prepare(
            "SELECT name, language, part_of_speech, definition FROM words WHERE name LIKE ?",
        )
        .unwrap()
    } else {
        conn.prepare("SELECT name, language, part_of_speech, definition FROM words WHERE name = ?")
            .unwrap()
    };

    let search_word = if search_partial {
        format!("%{}%", word)
    } else {
        word.to_string()
    };

    let word_iter = stmt
        .query_map(&[&search_word], |row| {
            Ok(Meaning {
                name: row.get(0).unwrap(),
                language: row.get(1).unwrap(),
                part_of_speech: row.get(2).unwrap(),
                definition: row.get(3).unwrap(),
            })
        })
        .unwrap();

    let mut words: WordMap = BTreeMap::new();

    for meaning in word_iter {
        let meaning = meaning.unwrap();
        words
            .entry(meaning.name)
            .or_insert(BTreeMap::new())
            .entry(meaning.language)
            .or_insert(BTreeMap::new())
            .entry(meaning.part_of_speech)
            .or_insert(Vec::new())
            .push(meaning.definition);
    }
    Box::new(words)
}

// TODO: Actually expand templates. This is very hard because Wikitext templates have a bunch of
// functions and often call out into Lua code.
// https://www.mediawiki.org/wiki/Help:Extension:ParserFunctions
// https://www.mediawiki.org/wiki/Extension:Scribunto
#[allow(dead_code)]
fn expand_template(conn: &Connection, args: &[&str]) -> String {
    fn get_template_content(conn: &Connection, name: &str) -> String {
        let result = conn.query_row(
            "SELECT content FROM templates WHERE name = ?1",
            &[&name],
            |row| row.get(0),
        );
        println!("{}", name);
        result.unwrap()
    }
    get_template_content(conn, args[0])
}
#[warn(dead_code)]

// For now, we just hardcode a couple common templates.
fn replace_template(_conn: &Connection, caps: &Captures) -> String {
    let s = caps.get(1).unwrap().as_str();
    let elems: Vec<&str> = s.split('|').collect();
    //match elems[0] {
    //    _ => expand_template(conn, &elems)
    //}
    match elems[0] {
        "," => ",".to_owned(),
        "ngd" | "unsupported" | "non-gloss definition" => elems[1].to_owned(),
        "alternative form of" => format!("Alternative form of {}", elems[1]),
        "ja-romanization of" => format!("Rōmaji transcription of {}", elems[1]),
        "sumti" => format!("x{}", elems[1]),
        "ja-def" => format!("{}:", elems[1]),
        "qualifier" => format!("({})", elems[1]),
        "lb" => format!("({})", elems[2]),
        "m" | "l" => elems[2].to_owned(),
        _ => caps.get(0).unwrap().as_str().to_owned(),
    }
}

fn print_words<F>(words: &WordMap, mut format: F)
where
    F: FnMut(&str) -> String,
{
    let textwrap_opts = textwrap::Options::new(80)
        .initial_indent("    ")
        .subsequent_indent("      ");

    for (name, langs) in words {
        for (lang, poses) in langs {
            println!("{}\n", lang.green().bold());
            let joined_pos = poses
                .keys()
                .map(|k| k.to_owned())
                .collect::<Vec<String>>()
                .join(", ");
            println!("  {} ({})\n", name.bold(), joined_pos);
            for (i, (pos, defns)) in poses.iter().enumerate() {
                println!("  {}", pos.white());
                for (j, defn) in defns.iter().enumerate() {
                    let defn = format(format!("{}. {}", j + 1, defn).as_ref());
                    let defn = textwrap::fill(&defn, &textwrap_opts);
                    println!("{}", defn);
                    if j < defns.len() - 1 || i < poses.len() {
                        println!()
                    }
                }
            }
        }

        if langs.len() == 0usize {
            println!("No results found.");
        }
    }
}

fn main() {
    let mut sqlite_path = dirs::data_dir().unwrap();
    sqlite_path.push("define3");
    sqlite_path.push("define3.sqlite3");

    let mut search_partial = false;

    let args: Vec<String> = env::args().collect();
    let mut opts = Options::new();
    opts.optflag("h", "help", "Print this help text");
    opts.optflag("r", "raw", "Don't expand wiki templates");
    opts.optopt("l", "language", "Only print this language", "LANG");
    opts.optopt(
        "d",
        "database",
        format!(
            "Database file path\n[default: {}]",
            sqlite_path.to_string_lossy()
        )
        .as_ref(),
        "FILE",
    );
    opts.optflag(
        "p",
        "partial",
        "Search database for words matching any part\n[default: false]",
    );

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            println!("{}", f.to_string());
            exit(1);
        }
    };

    if matches.opt_present("h") || matches.free.len() == 0 {
        let brief = format!("Usage: {} [options] WORD", args[0]);
        print!("{}", opts.usage(&brief));
        return;
    }
    if matches.opt_present("d") {
        sqlite_path = PathBuf::from(matches.opt_str("d").unwrap());
    }
    if matches.opt_present("p") {
        search_partial = true;
    }

    // TODO: We currently support nested templates in a very bad way. We expand templates in
    // layers, most deeply nested first, and we do this by excluding curly braces in the regex.
    // Should eventually use a more legit parser (nom maybe?)
    let re_template = Regex::new(r"\{\{(?P<text>(?s:[^\{])*?)\}\}").unwrap();

    let conn = Connection::open(sqlite_path).unwrap();

    let all_langs = *get_defns_by_lang(&conn, &matches.free.join(" "), search_partial);
    let langs = match matches.opt_str("l") {
        None => all_langs,
        Some(lang) => {
            let mut result = BTreeMap::new();
            for &result_for_lang in all_langs.get(&lang).iter() {
                result.insert(lang.clone(), result_for_lang.clone());
            }
            result
        }
    };
    print_words(&langs, |s| {
        let replace_template = |caps: &Captures| -> String { replace_template(&conn, caps) };
        let mut result = s.to_owned();
        if !matches.opt_present("r") {
            loop {
                let result_ = re_template
                    .replace_all(&result, &replace_template)
                    .to_string();
                //println!("{}", result_);
                if result == result_ {
                    break;
                }
                result = result_;
            }
        }
        result
    });
}
