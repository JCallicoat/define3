extern crate quick_xml;
use crate::parse_xml::quick_xml::{events::Event, Reader};

use std::io::BufRead;
use std::path::Path;

use crate::Page;

fn parse_revision<B: BufRead>(reader: &mut Reader<B>) -> Option<String> {
    let mut buf = Vec::new();
    let mut result = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"text" => {
                let mut buf = Vec::new();
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Text(e)) => {
                        let text = e.unescape().unwrap().to_string();
                        result = Some(text);
                    }
                    _ => (),
                }
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"revision" => break,
            _ => (),
        }
    }
    result
}

pub fn parse_page<B: BufRead>(mut reader: &mut Reader<B>) -> Option<Page> {
    let mut buf = Vec::new();
    let mut title = None;
    let mut content = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let mut buf = Vec::new();
                match e.name().as_ref() {
                    b"title" => match reader.read_event_into(&mut buf) {
                        Ok(Event::Text(e)) => title = Some(e.unescape().unwrap().to_string()),
                        _ => (),
                    },
                    b"revision" => {
                        content = parse_revision(&mut reader);
                    }
                    _ => (),
                }
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"page" => break,
            _ => (),
        }
    }
    // and_then is a poor name for >>=
    title.and_then(|title| content.map(|content| Page { title, content }))
}

pub fn for_pages<F>(filename: &str, mut f: F)
where
    F: FnMut(Page) -> (),
{
    let mut buf = Vec::new();
    let mut reader = Reader::from_file(Path::new(filename)).unwrap();
    'read_words: loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"page" => {
                    let page = parse_page(&mut reader);
                    page.map(|page| f(page));
                }
                _ => (),
            },
            Ok(Event::Eof) => break 'read_words,
            Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            _ => (),
        }
        buf.clear();
    }
}
