use clap::{Parser, ValueEnum};
use pulldown_cmark::{Event, Options, Parser as MarkdownParser, Tag, TagEnd};
use scraper::{node::Node, Html};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Markdown,
    Html,
}

#[derive(Debug, Parser)]
#[command(
    version,
    author,
    about = env!("CARGO_PKG_DESCRIPTION"),
    long_about = None
)]
struct Args {
    /// Input file; use - or omit to read stdin
    #[arg(short, long, default_value = "-")]
    input: PathBuf,

    /// Output file; use - to write stdout
    #[arg(short, long, default_value = "-")]
    output: PathBuf,

    /// Input format. Defaults to text; no heuristic auto-detection.
    #[arg(short, long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Print progress messages to stderr
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let input = read_input(&args.input)?;
    if args.verbose {
        eprintln!("Read {} bytes", input.len());
    }

    // Parsers consume UTF-8. Invalid input bytes are replaced here; URL bytes
    // decoded later are preserved exactly as requested.
    let input = String::from_utf8_lossy(&input);
    let mut extracted = Vec::with_capacity(input.len());

    match args.format {
        Format::Text => extracted.extend_from_slice(input.as_bytes()),
        Format::Markdown => extract_markdown(&input, &mut extracted),
        Format::Html => extract_html(&input, &mut extracted),
    }

    let cleaned = normalize_text(&extracted);
    write_output(&args.output, &cleaned)?;

    if args.verbose {
        eprintln!("Wrote {} bytes", cleaned.len());
    }
    Ok(())
}

fn read_input(path: &PathBuf) -> io::Result<Vec<u8>> {
    let reader: Box<dyn Read> = if path.as_os_str() == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(path)?)
    };

    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn write_output(path: &PathBuf, bytes: &[u8]) -> io::Result<()> {
    let writer: Box<dyn Write> = if path.as_os_str() == "-" {
        Box::new(io::stdout())
    } else {
        Box::new(File::create(path)?)
    };

    let mut writer = BufWriter::new(writer);
    writer.write_all(bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn extract_markdown(input: &str, out: &mut Vec<u8>) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;

    let parser = MarkdownParser::new_ext(input, options);
    let mut link_stack: Vec<LinkCapture> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_stack.push(LinkCapture {
                    url: dest_url.to_string(),
                    label_start: out.len(),
                    fallback: String::new(),
                });
            }
            Event::End(TagEnd::Link) => {
                if let Some(link) = link_stack.pop() {
                    append_url(out, &link.url);
                }
            }
            Event::Start(Tag::Image {
                             dest_url, title, ..
                         }) => {
                link_stack.push(LinkCapture {
                    url: dest_url.to_string(),
                    label_start: out.len(),
                    fallback: String::new(),
                });

                if !title.is_empty() {
                    link_stack.last_mut().unwrap().fallback = title.to_string();
                }
            }

            Event::End(TagEnd::Image) => {
                if let Some(link) = link_stack.pop() {
                    if out.len() == link.label_start && !link.fallback.is_empty() {
                        out.extend_from_slice(link.fallback.as_bytes());
                    }
                    append_url(out, &link.url);
                }
            }
            Event::Text(text) | Event::Code(text) | Event::Html(text) => {
                out.extend_from_slice(text.as_bytes());
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => out.push(b' '),
            Event::Start(Tag::CodeBlock(_)) | Event::End(TagEnd::CodeBlock) => out.push(b' '),
            // Formatting, heading, list, and paragraph events are structure,
            // not text, so they are omitted.
            _ => {}
        }
    }
}

#[derive(Default)]
struct LinkCapture {
    url: String,
    label_start: usize,
    fallback: String,
}

fn append_url(out: &mut Vec<u8>, url: &str) {
    if url.is_empty() || url.starts_with('#') {
        return;
    }

    out.extend_from_slice(b" (");
    out.extend_from_slice(&decode_url_bytes(url.as_bytes()));
    out.push(b')');
}

fn extract_html(input: &str, out: &mut Vec<u8>) {
    let document = Html::parse_document(input);

    for child in document.tree.root().children() {
        walk_html(child, out);
    }
}

fn walk_html<'a>(node: ego_tree::NodeRef<'a, Node>, out: &mut Vec<u8>) {
    match node.value() {
        Node::Text(text) => out.extend_from_slice(text.text.as_bytes()),
        Node::Element(element) => {
            let name = element.name();

            // Discard elements that do not represent visible article text.
            if matches!(
                name,
                "script"
                    | "style"
                    | "noscript"
                    | "template"
                    | "iframe"
                    | "svg"
                    | "canvas"
                    | "link"
                    | "meta"
                    | "head"
                    | "nav"
                    | "header"
                    | "footer"
                    | "aside"
            ) {
                return;
            }

            if name == "img" {
                let alt = element.attr("alt").unwrap_or("");
                let title = element.attr("title").unwrap_or("");
                let label = if !alt.is_empty() { alt } else { title };

                if !label.is_empty() {
                    out.extend_from_slice(label.as_bytes());
                }
                if let Some(src) = element.attr("src") {
                    append_url(out, src);
                }
                return;
            }

            if name == "a" {
                let start = out.len();
                for child in node.children() {
                    walk_html(child, out);
                }
                if let Some(href) = element.attr("href") {
                    if !href.starts_with('#') {
                        append_url(out, href);
                    } else if out.len() == start {
                        // Empty in-page jump link: discard it.
                        out.truncate(start);
                    }
                }
                return;
            }

            if is_block_element(name) {
                out.push(b' ');
            }
            for child in node.children() {
                walk_html(child, out);
            }
            if is_block_element(name) {
                out.push(b' ');
            }
        }
        _ => {}
    }
}

fn is_block_element(name: &str) -> bool {
    matches!(
        name,
        "address" | "article" | "blockquote" | "br" | "dd" | "div" | "dl"
            | "dt" | "fieldset" | "figcaption" | "figure" | "h1" | "h2"
            | "h3" | "h4" | "h5" | "h6" | "li" | "main" | "ol" | "p"
            | "pre" | "section" | "table" | "tbody" | "td" | "th" | "thead"
            | "tr" | "ul"
    )
}

fn normalize_text(input: &[u8]) -> Vec<u8> {
    let utf8_input = String::from_utf8_lossy(input);
    let decoded = html_escape::decode_html_entities(&utf8_input);
    let mut out = Vec::with_capacity(decoded.len());
    let mut pending_space = false;
    let mut i = 0;

    while i < decoded.len() {
        let byte = decoded.as_bytes()[i];

        // Remove ANSI terminal escape sequences.
        if byte == 0x1b {
            i = skip_ansi_escape(decoded.as_bytes(), i);
            pending_space = true;
            continue;
        }

        if byte.is_ascii_control() || byte == 0x7f {
            pending_space = true;
            i += 1;
            continue;
        }

        let rest = &decoded[i..];
        let ch = rest.chars().next().unwrap();
        let width = ch.len_utf8();

        if ch.is_whitespace() {
            pending_space = true;
            i += width;
            continue;
        }

        if pending_space && !out.is_empty() {
            out.push(b' ');
        }
        pending_space = false;

        match normalize_punctuation(ch) {
            Some(replacement) => {
                let mut buf = [0; 4];
                out.extend_from_slice(replacement.encode_utf8(&mut buf).as_bytes());
            }
            None => {
                // Preserve unrecognized Unicode and emoji as UTF-8.
                out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
            }
        }
        i += width;
    }

    while out.last() == Some(&b' ') {
        out.pop();
    }
    out
}

fn normalize_punctuation(ch: char) -> Option<char> {
    Some(match ch {
        // Curly/smart quotes and prime-like marks.
        '‘' | '’' | '‚' | '‛' | '′' => '\'',
        '“' | '”' | '„' | '‟' | '″' => '"',

        // Common dash and minus variants.
        '‐' | '‑' | '‒' | '–' | '—' | '―' | '−' => '-',

        // Comma-like punctuation.
        '，' | '､' | '﹐' | '、' => ',',

        // Period-like punctuation.
        '．' | '｡' | '。' | '․' => '.',

        // Keep common ASCII punctuation unchanged.
        _ if ch.is_ascii_punctuation() => ch,

        // Keep all other Unicode unchanged, including emoji.
        _ => return None,
    })
}

fn skip_ansi_escape(input: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    if i >= input.len() {
        return i;
    }

    // CSI sequence: ESC [ ... final-byte
    if input[i] == b'[' {
        i += 1;
        while i < input.len() {
            let byte = input[i];
            i += 1;
            if (0x40..=0x7e).contains(&byte) {
                break;
            }
        }
        return i;
    }

    // Other short escape: discard ESC and its following byte.
    (i + 1).min(input.len())
}

fn decode_url_bytes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        // Legacy JavaScript-style %uXXXX escape.
        if input[i] == b'%'
            && i + 5 < input.len()
            && input[i + 1] == b'u'
            && is_hex(input[i + 2])
            && is_hex(input[i + 3])
            && is_hex(input[i + 4])
            && is_hex(input[i + 5])
        {
            let code = ((hex(input[i + 2]) as u32) << 12)
                | ((hex(input[i + 3]) as u32) << 8)
                | ((hex(input[i + 4]) as u32) << 4)
                | hex(input[i + 5]) as u32;

            if let Some(ch) = char::from_u32(code) {
                let mut buffer = [0; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            } else {
                // Unpaired surrogate: preserve the escape literally.
                out.extend_from_slice(&input[i..i + 6]);
            }
            i += 6;
            continue;
        }

        // Standard percent escapes decode to their exact original byte,
        // whether or not that byte sequence is valid UTF-8.
        if input[i] == b'%'
            && i + 2 < input.len()
            && is_hex(input[i + 1])
            && is_hex(input[i + 2])
        {
            out.push((hex(input[i + 1]) << 4) | hex(input[i + 2]));
            i += 3;
            continue;
        }

        out.push(input[i]);
        i += 1;
    }

    out
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

fn hex(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}
