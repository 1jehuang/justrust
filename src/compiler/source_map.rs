//! Original-file byte provenance for the bounded source preparation transport.
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub lexical_path: PathBuf,
    pub contents: Arc<str>,
    pub line_starts: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub(crate) fn add(&mut self, path: PathBuf, contents: Arc<str>) -> FileId {
        let id = FileId(self.files.len() as u32);
        let mut line_starts = vec![0];
        line_starts.extend(
            contents
                .bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some((i + 1) as u32)),
        );
        self.files.push(SourceFile {
            lexical_path: path,
            contents,
            line_starts,
        });
        id
    }

    /// One-based line and Unicode scalar column, never a guessed expanded location.
    pub fn location(&self, span: SourceSpan) -> Option<(&std::path::Path, usize, usize)> {
        let file = self.files.get(span.file.0 as usize)?;
        let offset = span.lo as usize;
        if span.lo > span.hi
            || span.hi as usize > file.contents.len()
            || !file.contents.is_char_boundary(span.hi as usize)
            || !file.contents.is_char_boundary(offset)
        {
            return None;
        }
        let line = file
            .line_starts
            .partition_point(|&start| start <= span.lo)
            .checked_sub(1)?;
        let line_start = *file.line_starts.get(line)? as usize;
        if line_start > offset || !file.contents.is_char_boundary(line_start) {
            return None;
        }
        let column = file.contents[line_start..offset].chars().count() + 1;
        Some((&file.lexical_path, line + 1, column))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginSegment {
    pub prepared: Range<usize>,
    pub origin: SourceSpan,
    /// Rewritten braces or whitespace are anchored, not claimed as original bytes.
    pub synthetic: bool,
}

pub(crate) fn map_range(segments: &[OriginSegment], range: Range<usize>) -> Option<SourceSpan> {
    if range.start > range.end {
        return None;
    }
    // Prepared segments are monotonic, so token provenance lookup is logarithmic.
    let index = segments
        .partition_point(|s| s.prepared.start <= range.start)
        .checked_sub(1)?;
    let segment = segments.get(index)?;
    if segment.prepared.start > range.start
        || range.end > segment.prepared.end
        || segment.prepared.start > segment.prepared.end
        || segment.origin.lo > segment.origin.hi
    {
        return None;
    }
    if segment.synthetic {
        return Some(segment.origin);
    }
    if segment.prepared.len() != (segment.origin.hi - segment.origin.lo) as usize {
        return None;
    }
    Some(SourceSpan {
        file: segment.origin.file,
        lo: segment
            .origin
            .lo
            .checked_add(u32::try_from(range.start - segment.prepared.start).ok()?)?,
        hi: segment
            .origin
            .lo
            .checked_add(u32::try_from(range.end - segment.prepared.start).ok()?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_boundaries_choose_next_and_eof_chooses_end() {
        let segments = vec![
            OriginSegment {
                prepared: 0..2,
                origin: SourceSpan {
                    file: FileId(0),
                    lo: 10,
                    hi: 12,
                },
                synthetic: false,
            },
            OriginSegment {
                prepared: 2..4,
                origin: SourceSpan {
                    file: FileId(1),
                    lo: 20,
                    hi: 22,
                },
                synthetic: false,
            },
        ];
        assert_eq!(
            map_range(&segments, 2..2),
            Some(SourceSpan {
                file: FileId(1),
                lo: 20,
                hi: 20
            })
        );
        assert_eq!(
            map_range(&segments, 4..4),
            Some(SourceSpan {
                file: FileId(1),
                lo: 22,
                hi: 22
            })
        );
        assert!(map_range(&segments, 1..3).is_none());
    }

    #[test]
    fn malformed_affine_segments_and_line_tables_never_panic() {
        for (lo, hi) in [(u32::MAX, u32::MAX), (10, 9), (0, 1)] {
            let segment = OriginSegment {
                prepared: 0..2,
                origin: SourceSpan {
                    file: FileId(0),
                    lo,
                    hi,
                },
                synthetic: false,
            };
            assert!(map_range(&[segment], 0..2).is_none());
        }
        let mut sources = SourceMap::default();
        sources.add("unicode.rs".into(), Arc::from("α\r\nx"));
        let span = SourceSpan {
            file: FileId(0),
            lo: 4,
            hi: 5,
        };
        assert_eq!(sources.location(span).unwrap().1, 2);
        assert_eq!(sources.location(span).unwrap().2, 1);
        assert!(
            sources
                .location(SourceSpan {
                    file: FileId(0),
                    lo: 1,
                    hi: 2
                })
                .is_none()
        );
        sources.files[0].line_starts.clear();
        assert!(sources.location(span).is_none());
    }
}
