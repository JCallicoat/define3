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

fn make_ascii_titlecase(s: &mut str) {
    if let Some(r) = s.get_mut(0..1) {
        r.make_ascii_uppercase();
    }
}

fn parse_place(elems: Vec<&str>) -> String {
    let mut result = String::new();
    for elem in elems {
        if elem == "ucomm" {
            result.push_str("(Unincorporated Community) ")
        } else if elem == "CDP" || elem == "cdp" {
            result.push_str("(Census-Designated Place) ");
        } else if elem == "minor city" {
            result.push_str("(Minor City) ")
        } else if elem == "electoral division" {
            result.push_str("(Electoral Division) ");
        } else if elem.starts_with("seat=") {
            result.push_str(format!(" (Seat {})", elem.split("=").last().unwrap()).as_ref());
        } else if elem.starts_with("One of") {
            result.push_str(
                parse_place(
                    elem.replace("<<", "")
                        .replace(">>", "")
                        .split(" ")
                        .collect(),
                )
                .as_ref(),
            );
        } else if elem.starts_with("in ") || elem == "and" {
            result.push_str(format!("{} ", elem).as_ref());
        } else if elem.starts_with("c/") {
            result.push_str(format!("{}", elem.replace("c/", "")).as_ref());
        } else if elem.starts_with("cc/") {
            result.push_str(format!("{}", elem.replace("cc/", "")).as_ref());
        } else if elem.starts_with("s/") {
            result.push_str(format!("{}, ", elem.replace("s/", "")).as_ref());
        } else if elem.starts_with("co/") {
            result.push_str(format!("{}, ", elem.replace("co/", "")).as_ref());
        } else if elem.starts_with("state/") {
            let mut state = elem.replace("state/", "");
            make_ascii_titlecase(state.as_mut());
            result.push_str(format!("{}, ", state).as_ref());
        } else if elem.starts_with("city/") {
            let mut city = elem.replace("city/", "");
            make_ascii_titlecase(city.as_mut());
            result.push_str(format!("{}, ", city).as_ref());
        } else if elem.starts_with("town/") {
            let mut town = elem.replace("town/", "");
            make_ascii_titlecase(town.as_mut());
            result.push_str(format!("{}, ", town).as_ref());
        } else if elem.starts_with("prefecture:") {
            let mut prefecture = elem.replace("prefecture:", "").replace("Suf/", "");
            make_ascii_titlecase(prefecture.as_mut());
            result.push_str(format!("{} prefecture, ", prefecture).as_ref());
        } else if elem.starts_with("ar:") {
            let mut ar = elem.replace("ar:", "").replace("Suf/", "");
            make_ascii_titlecase(ar.as_mut());
            result.push_str(format!("{} Autonomous Region, ", ar).as_ref());
        } else {
            result.push_str(format!("{} ", elem).as_ref());
        }
    }
    result.trim_end().replace(",,", ",").to_string()
}

// For now, we just hardcode a couple common templates.
fn replace_template(_conn: &Connection, caps: &Captures) -> String {
    let s = caps.get(1).unwrap().as_str();
    let mut elems: Vec<&str> = s.split('|').collect();
    //match elems[0] {
    //    _ => expand_template(conn, &elems)
    //}
    match elems[0] {
        "," => ",".to_owned(),
        "1" | "cap" => {
            let mut title = elems[1].to_owned();
            make_ascii_titlecase(title.as_mut());
            title
        }
        "ngd" | "unsupported" | "non-gloss definition" | "gloss" => elems[1].to_owned(),
        "alternative form of" => format!("Alternative form of {}", elems[2]),
        "alt form" => format!("Alternative form of {}", elems[2]),
        "ja-romanization of" => format!("Rōmaji transcription of {}", elems[1]),
        "sumti" => format!("x{}", elems[1]),
        "ja-def" => format!("{}:", elems[1]),
        "qualifier" => format!("({})", elems[1]),
        "lb" => format!(
            "({})",
            elems[2..]
                .iter()
                .filter(|e| **e != "_")
                .copied()
                .collect::<Vec<&str>>()
                .join(", ")
        ),
        "q" => format!("({})", elems[1]),
        "c" | "m" | "l" | "w" => elems[2].to_owned(),
        "senseid" => String::new(),
        "alternative case form of" => format!("Alternative case form of {}", elems[2]),
        "plural of" => format!("Plural of {}", elems[2]),
        "infl of" => format!("Inflected form of {}", elems[2]),
        "syn of" | "synonym of" => format!("Synonym of {}", elems[2]),
        "acronym of" => format!("Acronym of {}", elems[2]),
        "initialism of" => format!("Initialism of {}", elems[2]),
        "abbreviation of" => format!("Abbreviation of {}", elems[2]),
        "clipping of" => format!("Clipping of {}", elems[2]),
        "surname" => "Surname".to_string(),
        "given name" => "Given name".to_string(),
        "defdate" => format!("[{}]", elems[1]),
        "place" => format!("(Place) {}", parse_place(elems[2..].to_vec())),
        "taxfmt" => format!("{}", elems[1..elems.len() - 1].join(" ")),
        "alt sp" => {
            let place = elems[3].replace("t=", "").clone();
            elems[3] = place.as_ref();
            format!(
                "Alt spelling of {} {}",
                elems[2],
                parse_place(elems[3..].to_vec())
            )
        }
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
