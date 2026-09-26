//! Mechanical integrity gate for docs/assets/icg-demo.gif, and the
//! executable definition of the workflow that regenerates it (bead
//! irrevers-dab6b05d; irrevers-07968bd2 closed the review-only half and
//! deferred this one).
//!
//! The regeneration workflow, end to end:
//!   1. a change moves what the demo shows -> demo.sh,
//!      tests/demo_verdict_regression_tests.rs' pinned matrix, and the GIF
//!      change together;
//!   2. recapture per the recipe in demo.sh's header: cargo build
//!      --release, prepend the release dir to PATH, `vhs
//!      docs/assets/demo.tape` -- the tape drives
//!      `bash docs/assets/demo.sh` against the repo's real packs/, so the
//!      GIF stays imagery of real output;
//!   3. update the byte-size note at the bottom of demo.sh in the same
//!      commit as the GIF;
//!   4. let the gates hold that discipline: scripts/check-doc-assets (in
//!      the DoD) fails a missing, emptied, wrong-magic, or trailer-less
//!      asset, the verdict suite pins demo.sh's behaviour, and this file
//!      pins the committed asset itself.
//!
//! Why this file has to parse the GIF rather than trust the trailer check:
//! a trailer proves the stream was finalized, not that it finished. A
//! capture that died three verdicts early still writes a clean 0x3B on
//! vhs's error path or a later aborted rerun. So the parse counts frames
//! and sums their animation delays, and the totals are pinned to what
//! docs/assets/demo.tape asks for -- parsed from the tape, not hardcoded,
//! so a legitimate tape edit moves the expectations instead of fighting
//! them. A GIF whose capture predates a tape or demo change, or whose run
//! was cut short, lands outside the window and fails with the recipe in
//! the message.
//!
//! Honest limit: none of this reads pixels. "Consistent with the pinned
//! verdict regression test" is enforced as (a) the tape demonstrably still
//! drives demo.sh, whose inputs that suite pins against the real binary,
//! and (b) the capture window covering the tape's whole runtime, so every
//! pinned verdict's frames exist in the committed stream. Pixel truth is
//! what the recapture recipe is for; these guards make silent drift
//! impossible rather than re-reviewing the image.

use std::path::PathBuf;

/// Reused test binaries bake `CARGO_MANIFEST_DIR` of a dead extraction (see
/// documentation_consistency_tests::audited_checkout); prefer the runtime
/// cwd, which cargo sets to the package root of the tree under test.
fn repo_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn gif_bytes() -> Vec<u8> {
    std::fs::read(repo_root().join("docs").join("assets").join("icg-demo.gif")).expect(
        "docs/assets/icg-demo.gif should be readable -- README.md embeds \
         it as the hero demo, so a missing or unreadable asset is a broken \
         front page, not a skipped test",
    )
}

fn tape_text() -> String {
    std::fs::read_to_string(
        repo_root().join("docs").join("assets").join("demo.tape"),
    )
    .expect("docs/assets/demo.tape should be readable -- it is the documented regeneration source of the GIF")
}

/// What one parsed GIF stream exposes, in the units the pins need.
#[derive(Debug)]
struct ParsedGif {
    /// Logical screen descriptor size, the render geometry vhs's
    /// `Set Width`/`Set Height` produce.
    screen: (u16, u16),
    /// Image descriptors (frames) in the stream.
    frames: usize,
    /// Sum of every Graphic Control Extension delay in 10 ms units --
    /// what a player would play, and therefore how much of the demo run
    /// the capture actually covers.
    delay_cs: u64,
    /// Byte index of the 0x3B trailer.
    trailer_at: usize,
}

/// Walk the GIF block structure. Anything but a well-formed stream ending
/// in the trailer is an Err naming the first broken thing -- a capture
/// killed mid-run dies in here, not in an assert far from the cause.
fn parse_gif(bytes: &[u8]) -> Result<ParsedGif, String> {
    if bytes.len() < 13 {
        return Err(format!(
            "{} bytes is shorter than the 13-byte GIF header",
            bytes.len()
        ));
    }
    if &bytes[..6] != b"GIF89a" && &bytes[..6] != b"GIF87a" {
        return Err(format!("bad magic {:?}", &bytes[..6]));
    }
    let screen = (
        u16::from(bytes[6]) | (u16::from(bytes[7]) << 8),
        u16::from(bytes[8]) | (u16::from(bytes[9]) << 8),
    );
    let mut i = 13usize;
    if bytes[10] & 0x80 != 0 {
        // Global color table: 3 bytes per entry, 2^(n+1) entries.
        i += 3 * (1usize << (u32::from(bytes[10] & 0x07) + 1));
    }

    let mut frames = 0usize;
    let mut delay_cs = 0u64;
    // A Graphic Control Extension precedes and applies to the frame that
    // follows it; hold its delay until that frame arrives.
    let mut delay_for_next_frame = 0u64;

    loop {
        let Some(&block) = bytes.get(i) else {
            return Err("stream ended without the 0x3B trailer -- truncated capture".to_string());
        };
        match block {
            0x3B => {
                return Ok(ParsedGif {
                    screen,
                    frames,
                    delay_cs,
                    trailer_at: i,
                });
            }
            0x21 => {
                let Some(&label) = bytes.get(i + 1) else {
                    return Err("extension label past EOF".to_string());
                };
                let mut j = i + 2;
                if label == 0xF9 {
                    // Graphic Control Extension: exactly one 4-byte
                    // sub-block -- packed, delay u16 LE, transparent
                    // index. j sits on the length byte, so the delay is
                    // at j+2..j+4.
                    let Some(&n) = bytes.get(j) else {
                        return Err("GCE length past EOF".to_string());
                    };
                    if n != 4 {
                        return Err(format!("GCE sub-block is {n} bytes, expected 4"));
                    }
                    let Some(d) = bytes.get(j + 2..j + 4) else {
                        return Err("GCE delay past EOF".to_string());
                    };
                    delay_for_next_frame = u64::from(d[0]) | (u64::from(d[1]) << 8);
                }
                j = skip_sub_blocks(bytes, j)?;
                i = j;
            }
            0x2C => {
                let Some(desc) = bytes.get(i + 1..i + 10) else {
                    return Err("image descriptor past EOF".to_string());
                };
                let packed = desc[8];
                let mut j = i + 10;
                if packed & 0x80 != 0 {
                    // Local color table, same shape as the global one.
                    j += 3 * (1usize << (u32::from(packed & 0x07) + 1));
                }
                let Some(&_lzw_min_code_size) = bytes.get(j) else {
                    return Err("LZW minimum code size past EOF".to_string());
                };
                j = skip_sub_blocks(bytes, j + 1)?;
                delay_cs += delay_for_next_frame;
                delay_for_next_frame = 0;
                frames += 1;
                i = j;
            }
            other => {
                return Err(format!(
                    "unknown block 0x{other:02X} at byte {i} -- not a parseable GIF stream"
                ));
            }
        }
    }
}

/// Advance past a chain of data sub-blocks (len byte, then that many
/// bytes, terminated by a zero length).
fn skip_sub_blocks(bytes: &[u8], mut j: usize) -> Result<usize, String> {
    loop {
        let Some(&n) = bytes.get(j) else {
            return Err("sub-block chain ran past EOF -- truncated capture".to_string());
        };
        j += 1;
        if n == 0 {
            return Ok(j);
        }
        j += usize::from(n);
        if j > bytes.len() {
            return Err("sub-block data runs past EOF -- truncated capture".to_string());
        }
    }
}

/// The tape facts the pins are stated against: the render geometry and
/// every `Sleep`, summed into seconds. Parsed, not hardcoded, so editing
/// the tape legitimately moves the expectations.
struct TapeSpec {
    width: u32,
    height: u32,
    sleep_seconds: f64,
}

fn parse_tape(text: &str) -> Result<TapeSpec, String> {
    let mut width = None;
    let mut height = None;
    let mut sleep_seconds = 0.0f64;
    let mut sleeps = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Set Width ") {
            width = Some(rest.trim().parse::<u32>().map_err(|_| {
                format!("unparseable `Set Width {rest}` -- the tape parser rotted")
            })?);
        } else if let Some(rest) = line.strip_prefix("Set Height ") {
            height = Some(rest.trim().parse::<u32>().map_err(|_| {
                format!("unparseable `Set Height {rest}` -- the tape parser rotted")
            })?);
        } else if let Some(rest) = line.strip_prefix("Sleep ") {
            let rest = rest.trim();
            let digits_end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            let (value, unit) = rest.split_at(digits_end);
            let secs: f64 = value
                .parse()
                .map_err(|_| format!("unparseable `Sleep {rest}`"))?;
            sleep_seconds += match unit {
                "s" => secs,
                "ms" => secs / 1000.0,
                "" => secs,
                other => return Err(format!("unknown Sleep unit {other:?} in `Sleep {rest}`")),
            };
            sleeps += 1;
        }
    }
    let width = width.ok_or("demo.tape has no `Set Width` -- the tape parser rotted")?;
    let height = height.ok_or("demo.tape has no `Set Height` -- the tape parser rotted")?;
    if sleeps == 0 {
        return Err("demo.tape has no `Sleep` -- the tape parser rotted".to_string());
    }
    Ok(TapeSpec {
        width,
        height,
        sleep_seconds,
    })
}

fn parse_committed_gif() -> (Vec<u8>, ParsedGif) {
    let bytes = gif_bytes();
    let recipe = "\nRecapture per the recipe in docs/assets/demo.sh's header, and update \
                  demo.sh's size note in the same commit.";
    let parsed = parse_gif(&bytes)
        .unwrap_or_else(|e| panic!("docs/assets/icg-demo.gif does not parse: {e}{recipe}"));
    (bytes, parsed)
}

/// Hand-built stream shaped like vhs's output: 1000x800 screen, no global
/// color table, one 40 ms GCE, one frame with a single LZW data sub-block,
/// trailer. What the parser must read back is stated inline, so a parser
/// regression (say, the GCE delay offset) moves a hand-pinned number and
/// not just an accident of the committed asset.
fn minimal_one_frame_stream() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"GIF89a");
    b.extend_from_slice(&1000u16.to_le_bytes()); // screen width
    b.extend_from_slice(&800u16.to_le_bytes()); // screen height
    b.push(0x00); // packed: no global color table
    b.push(0x00); // background color index
    b.push(0x00); // pixel aspect ratio
                  // Graphic Control Extension: block size 4, packed, delay 4 (40 ms),
                  // transparent index, block terminator.
    b.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00, 0x04, 0x00, 0x00, 0x00]);
    // Image descriptor at 0,0 sized 1000x800, no local color table.
    b.push(0x2C);
    b.extend_from_slice(&0u16.to_le_bytes()); // left
    b.extend_from_slice(&0u16.to_le_bytes()); // top
    b.extend_from_slice(&1000u16.to_le_bytes()); // width
    b.extend_from_slice(&800u16.to_le_bytes()); // height
    b.push(0x00); // packed: no local color table, not interleaved
    b.push(0x02); // LZW minimum code size
    b.extend_from_slice(&[0x02, 0x44, 0x01]); // one data sub-block
    b.push(0x00); // LZW data terminator
    b.push(0x3B); // trailer
    b
}

/// The parser earns its keep on synthetic streams, not only on the one
/// committed asset: if it stopped rejecting truncation, every pin below
/// would still hold and the gate would be theater.
#[test]
fn parser_reads_a_hand_built_minimal_stream() {
    let parsed = parse_gif(&minimal_one_frame_stream()).expect("a well-formed stream parses");
    assert_eq!(parsed.screen, (1000, 800));
    assert_eq!(parsed.frames, 1);
    assert_eq!(parsed.delay_cs, 4, "the GCE's 40 ms delay, in 10 ms units");
    assert_eq!(parsed.trailer_at, minimal_one_frame_stream().len() - 1);
}

#[test]
fn parser_rejects_a_stream_cut_before_the_trailer() {
    let mut cut = minimal_one_frame_stream();
    cut.pop(); // the trailer
    let err = parse_gif(&cut).expect_err("a stream with no trailer must not parse");
    assert!(
        err.contains("truncated"),
        "the error should name truncation, got: {err}"
    );
}

#[test]
fn parser_rejects_a_stream_cut_mid_frame_data() {
    let full = minimal_one_frame_stream();
    // Cut inside the frame's LZW data, well before the trailer: the shape
    // a capture killed by a signal leaves.
    let cut = &full[..full.len() - 3];
    let err = parse_gif(cut).expect_err("a stream cut mid-frame must not parse");
    assert!(
        err.contains("EOF") || err.contains("truncated"),
        "the error should say where the stream died, got: {err}"
    );
}

/// The stream must be complete: header, a well-formed block train, and a
/// trailer as the final byte. Anything trailing the trailer (a concatenated
/// capture, editor droppings) is drift too.
#[test]
fn committed_demo_gif_is_a_complete_stream() {
    let (bytes, parsed) = parse_committed_gif();
    assert_eq!(
        parsed.trailer_at + 1,
        bytes.len(),
        "the GIF trailer is not the final byte -- something follows the stream \
         (concatenated capture, editor droppings): {} bytes, trailer at {}",
        bytes.len(),
        parsed.trailer_at
    );
    assert!(
        parsed.frames > 0,
        "the GIF parses but holds zero frames -- that is a placeholder, not a capture"
    );
}

/// The capture's geometry is what the tape asked vhs for; anything else is
/// a capture from a different (older or edited) tape.
#[test]
fn demo_gif_geometry_matches_the_tape() {
    let tape = parse_tape(&tape_text())
        .unwrap_or_else(|e| panic!("docs/assets/demo.tape does not parse: {e}"));
    let (_, parsed) = parse_committed_gif();
    assert_eq!(
        (parsed.screen.0, parsed.screen.1),
        (
            u16::try_from(tape.width).expect("tape width over u16"),
            u16::try_from(tape.height).expect("tape height over u16")
        ),
        "the committed GIF's logical screen is {:?} but demo.tape asks vhs for \
         {}x{} -- the asset predates the current tape{}",
        parsed.screen,
        tape.width,
        tape.height,
        ". Recapture per the recipe in demo.sh's header."
    );
}

/// The capture must cover the pinned demo run. demo_verdict_regression_tests
/// pins that demo.sh's inputs still produce the verdicts README promises;
/// this pins that the committed GIF's frame train is long enough to contain
/// them. The window is generous around typing overhead at TypingSpeed 0ms
/// (~1% on the 32s tape) but rejects a run cut short by more than the
/// demo's final tail sleep -- and, from above, a tape shortened without a
/// recapture.
#[test]
fn demo_gif_covers_the_pinned_demo_runtime() {
    let tape = parse_tape(&tape_text())
        .unwrap_or_else(|e| panic!("docs/assets/demo.tape does not parse: {e}"));
    let (_, parsed) = parse_committed_gif();
    let duration_s = parsed.delay_cs as f64 / 100.0;
    assert!(
        duration_s >= 0.9 * tape.sleep_seconds,
        "the committed GIF plays for {duration_s:.2}s but demo.tape sleeps for \
         {:.2}s -- the capture is cut short and the later verdicts (the ones \
         README narrates last) are not in it. Recapture per the recipe in \
         demo.sh's header.",
        tape.sleep_seconds
    );
    assert!(
        duration_s <= 1.25 * tape.sleep_seconds,
        "the committed GIF plays for {duration_s:.2}s but demo.tape now asks \
         for only {:.2}s -- the tape was edited without a recapture. Recapture \
         per the recipe in demo.sh's header.",
        tape.sleep_seconds
    );
}

/// The workflow's chain is tape -> demo.sh -> real `icg check` output. The
/// verdict suite owns the second link; this owns the first -- if the tape
/// drifted to another script or another output path, the "regenerate per
/// the recipe" instructions everywhere would silently produce imagery of
/// something else.
#[test]
fn the_tape_still_drives_demo_sh_into_the_readme_asset() {
    let text = tape_text();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Output ") && l.ends_with("docs/assets/icg-demo.gif")),
        "demo.tape no longer writes docs/assets/icg-demo.gif -- every \
         regeneration instruction in demo.sh and these tests points at the \
         old path; update them together"
    );
    assert!(
        lines.iter().any(|l| l.contains("bash docs/assets/demo.sh")),
        "demo.tape no longer drives demo.sh -- the GIF would stop being \
         imagery of the script the verdict suite pins; update the tape and \
         the pinned matrix together"
    );
}
