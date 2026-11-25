//! A font file narrowed to the characters the interface draws, so that parsing it costs what those cost.
//!
//! `fontdue` parses, up front, the outline of every glyph the font's character map reaches: ~2 900 glyphs and
//! ~8.5 MB of memory for an Inter-like UI font, of which the window's chrome draws a few hundred. The outlines
//! are left as they are; only the map is replaced: a new `cmap` (one format 12 subtable with the characters
//! `keep` accepts) goes at the end of the file and the table directory points at it. Glyphs no character
//! reaches any more are never parsed. Composite glyphs still find their parts (they are read from `glyf` by
//! glyph id), and the glyph ids, advances and kerning do not change. TrueType and CFF outlines alike.
//!
//! No checksums are rewritten: nothing that reads the result checks them.

/// The characters the interface keeps from its font: Latin (with Latin-1 and Extended-A: the European languages),
/// Greek, Cyrillic, punctuation, currency, letter-like symbols, arrows, maths, technical symbols (⌘, ⏎),
/// geometric shapes and dingbats (✓, ✗). Anything else is drawn as the font's missing-glyph box, as a font
/// without it would. For Inter this is ~2.2 MB parsed instead of ~8.3 (Latin Extended-B and the Vietnamese
/// block would add another 1.3).
pub fn ui_char(c: u32) -> bool {
    matches!(c,
        0x20..=0x7e
        | 0xa0..=0x17f
        | 0x370..=0x3ff
        | 0x400..=0x4ff
        | 0x2000..=0x206f
        | 0x20a0..=0x20cf
        | 0x2100..=0x214f
        | 0x2190..=0x21ff
        | 0x2200..=0x22ff
        | 0x2300..=0x23ff
        | 0x25a0..=0x25ff
        | 0x2700..=0x27bf)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The font in `bytes` with a character map of only the characters `keep` accepts. None when it is not a
/// single font with a `cmap` this reads (a collection, a symbol font, a format other than 4 or 12): then the
/// caller uses the file as it is.
pub fn narrow(bytes: &[u8], keep: impl Fn(u32) -> bool) -> Option<Vec<u8>> {
    let tag = u32_at(bytes, 0)?;
    if !matches!(tag, 0x0001_0000 | 0x4f54_544f | 0x7472_7565) {
        return None; // not `\0\1\0\0`, `OTTO` or `true`: a collection, or not a font
    }
    let tables = u16_at(bytes, 4)? as usize;
    let rec = (0..tables).map(|i| 12 + 16 * i).find(|&r| bytes.get(r..r + 4) == Some(b"cmap"))?;
    let (off, len) = (u32_at(bytes, rec + 8)? as usize, u32_at(bytes, rec + 12)? as usize);
    let cmap = bytes.get(off..off.checked_add(len)?)?;
    let pairs = mappings(cmap, &keep)?;

    // The new table: version 0, one encoding record (Windows, Unicode full repertoire), one format 12 subtable
    // whose groups are the runs of consecutive characters with consecutive glyphs.
    let mut groups: Vec<(u32, u32, u32)> = Vec::new();
    for (c, g) in pairs {
        match groups.last_mut() {
            Some((start, end, g0)) if c == *end + 1 && g == *g0 + (c - *start) => *end = c,
            _ => groups.push((c, c, g)),
        }
    }
    let sub_len = 16 + 12 * groups.len();
    let mut table = Vec::with_capacity(12 + sub_len);
    table.extend_from_slice(&[0, 0, 0, 1, 0, 3, 0, 10]);
    table.extend_from_slice(&12u32.to_be_bytes());
    table.extend_from_slice(&[0, 12, 0, 0]);
    table.extend_from_slice(&(sub_len as u32).to_be_bytes());
    table.extend_from_slice(&0u32.to_be_bytes());
    table.extend_from_slice(&(groups.len() as u32).to_be_bytes());
    for (start, end, g) in groups {
        for v in [start, end, g] {
            table.extend_from_slice(&v.to_be_bytes());
        }
    }

    let mut out = Vec::with_capacity(bytes.len() + table.len() + 4);
    out.extend_from_slice(bytes);
    out.resize(out.len().next_multiple_of(4), 0);
    let at = out.len() as u32;
    out.extend_from_slice(&table);
    out[rec + 8..rec + 12].copy_from_slice(&at.to_be_bytes());
    out[rec + 12..rec + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    Some(out)
}

/// The (character, glyph) pairs of the best Unicode subtable of `cmap` that `keep` accepts, in character order,
/// without the unmapped ones.
fn mappings(cmap: &[u8], keep: &impl Fn(u32) -> bool) -> Option<Vec<(u32, u32)>> {
    let n = u16_at(cmap, 2)? as usize;
    // Format 12 over the full repertoire beats format 4 over the basic plane; Unicode platform or Windows Unicode.
    let mut best: Option<(u8, usize)> = None;
    for i in 0..n {
        let r = 4 + 8 * i;
        let (platform, encoding, off) = (u16_at(cmap, r)?, u16_at(cmap, r + 2)?, u32_at(cmap, r + 4)? as usize);
        let unicode = platform == 0 || (platform == 3 && matches!(encoding, 1 | 10));
        let rank = match u16_at(cmap, off) {
            Some(12) if unicode => 2,
            Some(4) if unicode => 1,
            _ => 0,
        };
        if rank > 0 && best.is_none_or(|(b, _)| rank > b) {
            best = Some((rank, off));
        }
    }
    let (rank, off) = best?;
    let sub = cmap.get(off..)?;
    let mut out = Vec::new();
    if rank == 2 {
        let groups = u32_at(sub, 12)? as usize;
        for i in 0..groups {
            let r = 16 + 12 * i;
            let (start, end, g0) = (u32_at(sub, r)?, u32_at(sub, r + 4)?, u32_at(sub, r + 8)?);
            // A malformed group cannot make the loop run for billions of characters: none of them is wanted.
            for c in start..=end.min(0x10ffff) {
                if keep(c) {
                    out.push((c, g0 + (c - start)));
                }
            }
        }
    } else {
        let segs = u16_at(sub, 6)? as usize / 2;
        let (ends, starts, deltas, ranges) = (14, 16 + 2 * segs, 16 + 4 * segs, 16 + 6 * segs);
        for s in 0..segs {
            let (end, start) = (u16_at(sub, ends + 2 * s)? as u32, u16_at(sub, starts + 2 * s)? as u32);
            let (delta, range) = (u16_at(sub, deltas + 2 * s)? as u32, u16_at(sub, ranges + 2 * s)? as usize);
            for c in start..=end {
                if c == 0xffff || !keep(c) {
                    continue;
                }
                let g = if range == 0 {
                    (c + delta) & 0xffff
                } else {
                    // `idRangeOffset` counts from its own position to the glyph id of `start`.
                    match u16_at(sub, ranges + 2 * s + range + 2 * (c - start) as usize)? as u32 {
                        0 => 0,
                        g => (g + delta) & 0xffff,
                    }
                };
                out.push((c, g));
            }
        }
    }
    out.retain(|&(_, g)| g != 0);
    out.sort_unstable();
    out.dedup_by_key(|p| p.0);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font of nothing but a table directory and a `cmap` holding one format 4 subtable for `segs`: (first
    /// character, last, glyph of the first) with consecutive glyphs, plus one segment through `idRangeOffset`
    /// mapping 'x' and 'y' to glyphs 90 and 0 (unmapped).
    fn font(segs: &[(u16, u16, u16)]) -> Vec<u8> {
        let mut sub: Vec<u16> = Vec::new();
        let n = segs.len() + 2; // the ranged segment and the closing 0xffff one
        let mut ends: Vec<u16> = segs.iter().map(|s| s.1).collect();
        let mut starts: Vec<u16> = segs.iter().map(|s| s.0).collect();
        let mut deltas: Vec<u16> = segs.iter().map(|s| s.2.wrapping_sub(s.0)).collect();
        let mut ranges: Vec<u16> = vec![0; segs.len()];
        ends.extend([b'y' as u16, 0xffff]);
        starts.extend([b'x' as u16, 0xffff]);
        deltas.extend([0, 1]);
        // From its own slot to the glyph array right after the closing segment's slot: two slots on.
        ranges.extend([4, 0]);
        sub.extend([4, 0, 0, 2 * n as u16, 0, 0, 0]);
        sub.extend(&ends);
        sub.push(0);
        sub.extend(&starts);
        sub.extend(&deltas);
        sub.extend(&ranges);
        sub.extend([90, 0]);
        let sub: Vec<u8> = sub.iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut cmap = vec![0, 0, 0, 1, 0, 3, 0, 1, 0, 0, 0, 12];
        cmap.extend(sub);
        let mut f = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        f.extend(b"cmap");
        f.extend([0; 4]);
        f.extend(28u32.to_be_bytes());
        f.extend((cmap.len() as u32).to_be_bytes());
        f.extend(cmap);
        f
    }

    /// The (character, glyph) pairs of a narrowed font's format 12 table.
    fn read(f: &[u8]) -> Vec<(u32, u32)> {
        let off = u32_at(f, 12 + 8).unwrap() as usize;
        mappings(&f[off..], &|_| true).unwrap()
    }

    #[test]
    fn only_the_kept_characters_stay_and_keep_their_glyphs() {
        let f = font(&[(b'a' as u16, b'f' as u16, 10), (0x2190, 0x2193, 50), (0x4e00, 0x4e02, 70)]);
        let all = mappings(&f[28..], &|_| true).unwrap();
        assert_eq!(all.len(), 6 + 4 + 3 + 1, "'y' maps to no glyph");
        let narrowed = narrow(&f, ui_char).unwrap();
        let kept = read(&narrowed);
        assert_eq!(kept, all.iter().copied().filter(|&(c, _)| ui_char(c)).collect::<Vec<_>>());
        assert!(kept.contains(&('c' as u32, 12)) && kept.contains(&('x' as u32, 90)) && kept.contains(&(0x2192, 52)));
        assert!(!kept.iter().any(|&(c, _)| c >= 0x4e00), "CJK is not the interface's");
        // The original bytes are untouched before the new table, which starts on a 4-byte boundary.
        assert_eq!(&narrowed[28..f.len()], &f[28..]);
        assert_eq!(u32_at(&narrowed, 20).unwrap() % 4, 0);
    }

    #[test]
    fn runs_of_consecutive_glyphs_become_one_group() {
        let f = font(&[(b'A' as u16, b'Z' as u16, 100), (b'0' as u16, b'9' as u16, 300)]);
        let n = narrow(&f, |c| c < 0x80).unwrap();
        let off = u32_at(&n, 20).unwrap() as usize;
        assert_eq!(read(&n).len(), 26 + 10 + 1);
        assert_eq!(u32_at(&n, off + 12 + 12), Some(3), "0-9, A-Z and x");
    }

    #[test]
    fn what_is_not_a_single_font_is_left_alone() {
        assert!(narrow(b"ttcf\0\0\0\0", ui_char).is_none());
        assert!(narrow(&[0, 1, 0, 0, 0, 0], ui_char).is_none(), "no cmap");
        assert!(narrow(&[], ui_char).is_none());
        let mut f = font(&[(b'a' as u16, b'c' as u16, 1)]);
        f.truncate(40);
        assert!(narrow(&f, ui_char).is_none(), "a cut file is refused, not read past its end");
    }
}
