//! Font data model and binary file format.
//!
//! # File format
//!
//! ```text
//! Offset  Size  Description
//! ──────  ────  ────────────────────────────────────────────────────────────
//!      0     4  Magic bytes: 0x46 0x4E 0x54 0x00  ("FNT\0")
//!      4     1  Glyph width  in pixels  (1–255)
//!      5     1  Glyph height in pixels  (1–255)
//!      6     1  Glyphs per row          (1–255)
//!      7     1  First glyph ASCII code  (e.g. 0x61 = 'a')
//!      8     2  Total number of glyphs  (little-endian u16)
//!     10     1  Flags: bit 0 → 0 = row-major, 1 = column-major
//!     11     1  Format version (current: 1)
//!     12+    N  Glyph pixel data (layout depends on the version, see below)
//! ```
//!
//! Within every glyph the pixels are numbered according to the encoding flag:
//! - **Row-major**: left-to-right, top-to-bottom.
//!   Bit `i` = pixel at `(i % width, i / width)`.
//! - **Column-major**: top-to-bottom, left-to-right.
//!   Bit `i` = pixel at `(i / height, i % height)`.
//!
//! Bits are packed least-significant-bit first: the first pixel of a byte is
//! stored in that byte's LSB.
//!
//! # Glyph data layout by version
//!
//! - **Version 0** (legacy): each glyph is packed independently and padded to a
//!   whole number of bytes — `ceil(width * height / 8)` bytes per glyph.  The
//!   spare bits at the end of every glyph are wasted.
//! - **Version 1** (current): every glyph's bits are concatenated into a single
//!   continuous stream, so padding happens only once, at the very end of the
//!   file — `ceil(total_glyphs * width * height / 8)` bytes total.  This is
//!   smaller, which matters when embedding fonts on constrained devices.
//!
//! Version-0 files still load (the reader unpacks them with the legacy layout);
//! [`Font::save`] always writes the current version, so opening an old file and
//! saving it upgrades it to version 1.

use std::io::{self, Read, Write};

/// Magic bytes that identify a valid font file.
pub const MAGIC: [u8; 4] = [b'F', b'N', b'T', 0x00];

/// Fixed size (in bytes) of the file header.
pub const HEADER_SIZE: usize = 12;

/// Font file format version written by [`Font::save`].  Bump this whenever the
/// on-disk layout changes so [`Font::load`] can branch on older versions.
pub const FORMAT_VERSION: u8 = 1;

/// Bit flag: column-major encoding.
pub const FLAG_COLUMN_MAJOR: u8 = 0b0000_0001;

/// A bitmap font composed of fixed-size glyphs.
#[derive(Clone, Debug)]
pub struct Font {
    /// Width of every glyph in pixels.
    pub width: u8,
    /// Height of every glyph in pixels.
    pub height: u8,
    /// How many glyphs fit in a single row of the glyph sheet.
    pub glyphs_per_row: u8,
    /// ASCII code of the first (index 0) glyph.
    pub first_glyph: u8,
    /// Total number of glyphs stored in the font.
    pub total_glyphs: u16,
    /// When `true` pixels are stored column-major; row-major otherwise.
    pub column_major: bool,
    /// Format version this font was loaded from (or [`FORMAT_VERSION`] for a
    /// font created in memory).  Informational only — [`Font::save`] always
    /// writes [`FORMAT_VERSION`].
    pub version: u8,
    /// Pixel data for each glyph.  `glyphs[i]` has `width * height` entries.
    pub glyphs: Vec<Vec<bool>>,
}

impl Default for Font {
    fn default() -> Self {
        Self::new(7, 10, 16, b' ', 95, false)
    }
}

impl Font {
    /// Create a new blank font with the given parameters.
    pub fn new(
        width: u8,
        height: u8,
        glyphs_per_row: u8,
        first_glyph: u8,
        total_glyphs: u16,
        column_major: bool,
    ) -> Self {
        let pixels = (width as usize) * (height as usize);
        let glyphs = vec![vec![false; pixels]; total_glyphs as usize];
        Self {
            width,
            height,
            glyphs_per_row,
            first_glyph,
            total_glyphs,
            column_major,
            version: FORMAT_VERSION,
            glyphs,
        }
    }

    /// Number of pixels (bits) in a single glyph.
    pub fn pixels_per_glyph(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    /// Total number of pixel cells across every glyph
    /// (`pixels_per_glyph * total_glyphs`).
    pub fn total_pixels(&self) -> usize {
        self.pixels_per_glyph() * self.total_glyphs as usize
    }

    /// Number of set (filled) pixel cells across every glyph.
    pub fn filled_pixels(&self) -> usize {
        self.glyphs.iter().flatten().filter(|&&p| p).count()
    }

    /// Total size in bytes of the glyph pixel-data array as written by
    /// [`Font::save`] (version 1: all glyph bits packed contiguously).
    pub fn data_size(&self) -> usize {
        (self.pixels_per_glyph() * self.total_glyphs as usize).div_ceil(8)
    }

    /// Total size in bytes of the serialised font file (header + glyph data).
    pub fn file_size(&self) -> usize {
        HEADER_SIZE + self.data_size()
    }

    /// Number of rows needed to display all glyphs at `glyphs_per_row` columns.
    pub fn rows(&self) -> u16 {
        if self.glyphs_per_row == 0 {
            return 0;
        }
        self.total_glyphs.div_ceil(self.glyphs_per_row as u16)
    }

    /// Return the character that corresponds to a glyph index, if the index
    /// is within the font's glyph range and maps to a printable ASCII character.
    pub fn glyph_char(&self, index: usize) -> Option<char> {
        if index >= self.total_glyphs as usize {
            return None;
        }
        let code = (self.first_glyph as usize).checked_add(index)?;
        if code > 127 {
            return None;
        }
        Some(code as u8 as char)
    }

    /// Read the value of a single pixel in a glyph.
    ///
    /// Returns `false` for out-of-bounds coordinates.
    pub fn get_pixel(&self, glyph_idx: usize, x: usize, y: usize) -> bool {
        let Some(glyph) = self.glyphs.get(glyph_idx) else {
            return false;
        };
        let w = self.width as usize;
        let h = self.height as usize;
        if x >= w || y >= h {
            return false;
        }
        glyph[self.pixel_index(w, h, x, y)]
    }

    /// Set or clear a single pixel in a glyph.  Out-of-bounds writes are
    /// silently ignored.
    pub fn set_pixel(&mut self, glyph_idx: usize, x: usize, y: usize, value: bool) {
        let w = self.width as usize;
        let h = self.height as usize;
        if glyph_idx >= self.glyphs.len() || x >= w || y >= h {
            return;
        }
        let idx = self.pixel_index(w, h, x, y);
        self.glyphs[glyph_idx][idx] = value;
    }

    /// Toggle a single pixel and return the new value.  Returns `false` for
    /// out-of-bounds coordinates.
    #[cfg(test)]
    fn toggle_pixel(&mut self, glyph_idx: usize, x: usize, y: usize) -> bool {
        let current = self.get_pixel(glyph_idx, x, y);
        self.set_pixel(glyph_idx, x, y, !current);
        !current
    }

    /// Clear all pixels of a glyph.
    pub fn clear_glyph(&mut self, glyph_idx: usize) {
        if let Some(glyph) = self.glyphs.get_mut(glyph_idx) {
            glyph.fill(false);
        }
    }

    /// Bounding box of the set pixels in a glyph as
    /// `(min_x, min_y, max_x, max_y)`, or `None` if the glyph is empty.
    fn glyph_bounds(&self, glyph_idx: usize) -> Option<(usize, usize, usize, usize)> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for y in 0..h {
            for x in 0..w {
                if self.get_pixel(glyph_idx, x, y) {
                    bounds = Some(match bounds {
                        None => (x, y, x, y),
                        Some((min_x, min_y, max_x, max_y)) => {
                            (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                        }
                    });
                }
            }
        }
        bounds
    }

    /// Shift all set pixels of a glyph by `(dx, dy)`.  Pixels shifted outside
    /// the glyph are discarded.
    fn shift_glyph(&mut self, glyph_idx: usize, dx: isize, dy: isize) {
        if (dx == 0 && dy == 0) || glyph_idx >= self.glyphs.len() {
            return;
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let old = self.glyphs[glyph_idx].clone();
        let mut shifted = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                if old[self.pixel_index(w, h, x, y)] {
                    let nx = x as isize + dx;
                    let ny = y as isize + dy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        shifted[self.pixel_index(w, h, nx as usize, ny as usize)] = true;
                    }
                }
            }
        }
        self.glyphs[glyph_idx] = shifted;
    }

    /// Horizontally centre the set pixels of a glyph.  Empty glyphs are
    /// left unchanged.  When the free space is odd the extra column goes to
    /// the right.
    pub fn center_glyph_horizontally(&mut self, glyph_idx: usize) {
        let Some((min_x, _, max_x, _)) = self.glyph_bounds(glyph_idx) else {
            return;
        };
        let w = self.width as usize;
        let content_w = max_x - min_x + 1;
        let target_min = (w - content_w) / 2;
        self.shift_glyph(glyph_idx, target_min as isize - min_x as isize, 0);
    }

    /// Vertically centre the set pixels of a glyph.  Empty glyphs are left
    /// unchanged.  When the free space is odd the extra row goes to the
    /// bottom.
    pub fn center_glyph_vertically(&mut self, glyph_idx: usize) {
        let Some((_, min_y, _, max_y)) = self.glyph_bounds(glyph_idx) else {
            return;
        };
        let h = self.height as usize;
        let content_h = max_y - min_y + 1;
        let target_min = (h - content_h) / 2;
        self.shift_glyph(glyph_idx, 0, target_min as isize - min_y as isize);
    }

    fn pixel_index(&self, w: usize, h: usize, x: usize, y: usize) -> usize {
        if self.column_major {
            x * h + y
        } else {
            y * w + x
        }
    }

    /// Serialise the font to `writer` in the binary file format.
    pub fn save<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Header
        writer.write_all(&MAGIC)?;
        writer.write_all(&[
            self.width,
            self.height,
            self.glyphs_per_row,
            self.first_glyph,
        ])?;
        writer.write_all(&self.total_glyphs.to_le_bytes())?;
        let flags: u8 = if self.column_major { FLAG_COLUMN_MAJOR } else { 0 };
        writer.write_all(&[flags, FORMAT_VERSION])?; // flags + version byte

        // Glyph pixel data (version 1): every glyph's bits are concatenated
        // into one continuous LSB-first stream, so only the final byte can be
        // partially wasted.
        let pixels_per_glyph = self.pixels_per_glyph();
        let mut bytes = vec![0u8; self.data_size()];
        let mut bit = 0usize;
        for glyph in &self.glyphs {
            for i in 0..pixels_per_glyph {
                if glyph.get(i).copied().unwrap_or(false) {
                    bytes[bit / 8] |= 1 << (bit % 8);
                }
                bit += 1;
            }
        }
        writer.write_all(&bytes)?;
        Ok(())
    }

    /// Deserialise a font from `reader`.
    pub fn load<R: Read>(reader: &mut R) -> io::Result<Self> {
        let mut header = [0u8; HEADER_SIZE];
        reader.read_exact(&mut header)?;

        if header[0..4] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a valid font file (bad magic bytes)",
            ));
        }

        let width = header[4];
        let height = header[5];
        let glyphs_per_row = header[6];
        let first_glyph = header[7];
        let total_glyphs = u16::from_le_bytes([header[8], header[9]]);
        let flags = header[10];
        let column_major = (flags & FLAG_COLUMN_MAJOR) != 0;
        let version = header[11];

        // Refuse files written by a newer format than we understand.  As new
        // versions are added, branch the parsing below on `version`.
        if version > FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported font format version {version} \
                     (this build supports up to {FORMAT_VERSION})"
                ),
            ));
        }

        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "glyph dimensions must be at least 1×1",
            ));
        }

        let pixels_per_glyph = (width as usize) * (height as usize);
        let glyphs = match version {
            0 => Self::read_glyphs_v0(reader, pixels_per_glyph, total_glyphs)?,
            _ => Self::read_glyphs_v1(reader, pixels_per_glyph, total_glyphs)?,
        };

        Ok(Self {
            width,
            height,
            glyphs_per_row,
            first_glyph,
            total_glyphs,
            column_major,
            version,
            glyphs,
        })
    }

    /// Read version-0 glyph data: each glyph padded to a whole number of bytes.
    fn read_glyphs_v0<R: Read>(
        reader: &mut R,
        pixels_per_glyph: usize,
        total_glyphs: u16,
    ) -> io::Result<Vec<Vec<bool>>> {
        let bytes_per_glyph = pixels_per_glyph.div_ceil(8);
        let mut glyphs = Vec::with_capacity(total_glyphs as usize);
        for _ in 0..total_glyphs {
            let mut raw = vec![0u8; bytes_per_glyph];
            reader.read_exact(&mut raw)?;
            let mut pixels = vec![false; pixels_per_glyph];
            for (i, p) in pixels.iter_mut().enumerate() {
                *p = (raw[i / 8] & (1 << (i % 8))) != 0;
            }
            glyphs.push(pixels);
        }
        Ok(glyphs)
    }

    /// Read version-1 glyph data: all glyph bits packed contiguously.
    fn read_glyphs_v1<R: Read>(
        reader: &mut R,
        pixels_per_glyph: usize,
        total_glyphs: u16,
    ) -> io::Result<Vec<Vec<bool>>> {
        let total_bits = pixels_per_glyph * total_glyphs as usize;
        let mut raw = vec![0u8; total_bits.div_ceil(8)];
        reader.read_exact(&mut raw)?;
        let mut glyphs = Vec::with_capacity(total_glyphs as usize);
        let mut bit = 0usize;
        for _ in 0..total_glyphs {
            let mut pixels = vec![false; pixels_per_glyph];
            for p in pixels.iter_mut() {
                *p = (raw[bit / 8] & (1 << (bit % 8))) != 0;
                bit += 1;
            }
            glyphs.push(pixels);
        }
        Ok(glyphs)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Create a minimal font, serialise it, then deserialise and compare.
    #[test]
    fn round_trip_empty_font() {
        let original = Font::new(8, 8, 16, b'a', 4, false);
        let mut buf = Vec::new();
        original.save(&mut buf).unwrap();

        let loaded = Font::load(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(loaded.width, original.width);
        assert_eq!(loaded.height, original.height);
        assert_eq!(loaded.glyphs_per_row, original.glyphs_per_row);
        assert_eq!(loaded.first_glyph, original.first_glyph);
        assert_eq!(loaded.total_glyphs, original.total_glyphs);
        assert_eq!(loaded.column_major, original.column_major);
        assert_eq!(loaded.glyphs, original.glyphs);
    }

    /// Verify that pixel data is preserved through a save/load cycle.
    #[test]
    fn round_trip_pixel_data() {
        let mut font = Font::new(5, 7, 8, b'A', 2, false);
        // Set a checkerboard pattern in glyph 0
        for y in 0..7 {
            for x in 0..5 {
                font.set_pixel(0, x, y, (x + y) % 2 == 0);
            }
        }
        // Set all pixels in glyph 1
        for y in 0..7 {
            for x in 0..5 {
                font.set_pixel(1, x, y, true);
            }
        }

        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        let loaded = Font::load(&mut Cursor::new(&buf)).unwrap();

        assert_eq!(loaded.glyphs, font.glyphs);
    }

    /// Column-major fonts must survive the round-trip with the flag preserved.
    #[test]
    fn round_trip_column_major() {
        let mut font = Font::new(4, 8, 4, b'0', 3, true);
        font.set_pixel(0, 0, 0, true);
        font.set_pixel(0, 3, 7, true);

        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        let loaded = Font::load(&mut Cursor::new(&buf)).unwrap();

        assert!(loaded.column_major);
        assert!(loaded.get_pixel(0, 0, 0));
        assert!(loaded.get_pixel(0, 3, 7));
        assert!(!loaded.get_pixel(0, 1, 1));
    }

    /// Glyphs are saved in ascending character order so the first glyph maps
    /// to `first_glyph` and each subsequent one maps to the next codepoint.
    #[test]
    fn glyph_ordering() {
        let mut font = Font::new(3, 3, 8, b'a', 3, false);
        // Mark each glyph with a unique top-left pixel pattern
        font.set_pixel(0, 0, 0, true);  // 'a' glyph
        font.set_pixel(1, 1, 0, true);  // 'b' glyph
        font.set_pixel(2, 2, 0, true);  // 'c' glyph

        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        let loaded = Font::load(&mut Cursor::new(&buf)).unwrap();

        // 'a' -> index 0, top-left pixel set
        assert!(loaded.get_pixel(0, 0, 0));
        assert!(!loaded.get_pixel(0, 1, 0));
        // 'b' -> index 1, second pixel of first row set
        assert!(!loaded.get_pixel(1, 0, 0));
        assert!(loaded.get_pixel(1, 1, 0));
        // 'c' -> index 2, third pixel of first row set
        assert!(loaded.get_pixel(2, 2, 0));
    }

    /// Reading a buffer with wrong magic must return an error.
    #[test]
    fn bad_magic_is_rejected() {
        let mut buf = vec![0u8; HEADER_SIZE + 8];
        buf[0] = b'X'; // corrupt magic
        let result = Font::load(&mut Cursor::new(&buf));
        assert!(result.is_err());
    }

    /// `save` must write the current format version into the header.
    #[test]
    fn save_writes_format_version() {
        let font = Font::new(4, 4, 4, b'a', 1, false);
        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        assert_eq!(buf[11], FORMAT_VERSION);
    }

    /// A file claiming a newer format version than we support must be rejected.
    #[test]
    fn newer_version_is_rejected() {
        let font = Font::new(4, 4, 4, b'a', 1, false);
        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        buf[11] = FORMAT_VERSION + 1; // pretend it's from a future build
        let result = Font::load(&mut Cursor::new(&buf));
        assert!(result.is_err());
    }

    /// Serialise `font` using the legacy version-0 layout (each glyph padded to
    /// a whole number of bytes) so we can test backward-compatible loading.
    fn encode_v0(font: &Font) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&[
            font.width,
            font.height,
            font.glyphs_per_row,
            font.first_glyph,
        ]);
        buf.extend_from_slice(&font.total_glyphs.to_le_bytes());
        let flags = if font.column_major { FLAG_COLUMN_MAJOR } else { 0 };
        buf.extend_from_slice(&[flags, 0]); // version byte = 0
        let pixels = font.pixels_per_glyph();
        let bytes_per_glyph = pixels.div_ceil(8);
        for glyph in &font.glyphs {
            let mut bytes = vec![0u8; bytes_per_glyph];
            for (i, &p) in glyph.iter().enumerate().take(pixels) {
                if p {
                    bytes[i / 8] |= 1 << (i % 8);
                }
            }
            buf.extend_from_slice(&bytes);
        }
        buf
    }

    /// Version-0 files must still load, exposing their version, and re-saving
    /// them upgrades to the smaller version-1 layout without losing pixels.
    #[test]
    fn loads_version_0_and_upgrades_on_save() {
        let mut original = Font::new(5, 7, 8, b'A', 3, false);
        original.set_pixel(0, 0, 0, true);
        original.set_pixel(1, 4, 6, true);
        original.set_pixel(2, 2, 3, true);

        let v0 = encode_v0(&original);
        assert_eq!(v0[11], 0);

        let loaded = Font::load(&mut Cursor::new(&v0)).unwrap();
        assert_eq!(loaded.version, 0);
        assert_eq!(loaded.glyphs, original.glyphs);

        // Re-saving writes the current version and a smaller payload.
        let mut v1 = Vec::new();
        loaded.save(&mut v1).unwrap();
        assert_eq!(v1[11], FORMAT_VERSION);
        assert!(v1.len() < v0.len());

        let reloaded = Font::load(&mut Cursor::new(&v1)).unwrap();
        assert_eq!(reloaded.version, FORMAT_VERSION);
        assert_eq!(reloaded.glyphs, original.glyphs);
    }

    /// A freshly saved font reports the current version when reloaded.
    #[test]
    fn round_trip_reports_current_version() {
        let font = Font::new(7, 10, 16, b' ', 95, false);
        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        let loaded = Font::load(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(loaded.version, FORMAT_VERSION);
    }

    /// A font with zero-dimension glyphs must be rejected.
    #[test]
    fn zero_dimension_rejected() {
        let mut buf = vec![0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&MAGIC);
        buf[4] = 0; // width = 0  ← invalid
        buf[5] = 8;
        let result = Font::load(&mut Cursor::new(&buf));
        assert!(result.is_err());
    }

    /// The size helpers must agree with the actual serialised byte count.
    #[test]
    fn size_helpers_match_serialised_output() {
        let font = Font::new(5, 7, 8, b'A', 26, false);
        assert_eq!(font.pixels_per_glyph(), 35); // 5 * 7
        // Version 1 packs all glyph bits contiguously: ceil(35 * 26 / 8) = 114.
        assert_eq!(font.data_size(), 114);
        assert_eq!(font.file_size(), HEADER_SIZE + 114);

        let mut buf = Vec::new();
        font.save(&mut buf).unwrap();
        assert_eq!(buf.len(), font.file_size());
    }

    #[test]
    fn rows_calculation() {
        let font = Font::new(8, 8, 16, b'a', 26, false);
        assert_eq!(font.rows(), 2); // ceil(26 / 16) = 2
    }

    #[test]
    fn glyph_char_mapping() {
        let font = Font::new(8, 8, 16, b'A', 26, false);
        assert_eq!(font.glyph_char(0), Some('A'));
        assert_eq!(font.glyph_char(25), Some('Z'));
        assert_eq!(font.glyph_char(26), None); // out of range
    }

    /// A 2-wide block at the left edge of an 8-wide glyph centres to x=3..4.
    #[test]
    fn center_horizontally() {
        let mut font = Font::new(8, 8, 16, b'a', 1, false);
        font.set_pixel(0, 0, 2, true);
        font.set_pixel(0, 1, 2, true);

        font.center_glyph_horizontally(0);

        assert!(!font.get_pixel(0, 0, 2));
        assert!(!font.get_pixel(0, 1, 2));
        assert!(font.get_pixel(0, 3, 2)); // (8 - 2) / 2 = 3
        assert!(font.get_pixel(0, 4, 2));
        // Row must be unchanged
        assert!(!font.get_pixel(0, 3, 1));
    }

    /// A single pixel at the bottom of an 8-tall glyph centres to y=3.
    #[test]
    fn center_vertically() {
        let mut font = Font::new(8, 8, 16, b'a', 1, false);
        font.set_pixel(0, 5, 7, true);

        font.center_glyph_vertically(0);

        assert!(!font.get_pixel(0, 5, 7));
        assert!(font.get_pixel(0, 5, 3)); // (8 - 1) / 2 = 3
        // Column must be unchanged
        assert!(!font.get_pixel(0, 4, 3));
    }

    /// Centring an empty glyph or an already-centred glyph is a no-op.
    #[test]
    fn center_noop_cases() {
        let mut font = Font::new(8, 8, 16, b'a', 1, false);
        font.center_glyph_horizontally(0);
        font.center_glyph_vertically(0);
        assert!(font.glyphs[0].iter().all(|&p| !p));

        font.set_pixel(0, 3, 3, true);
        font.set_pixel(0, 4, 4, true);
        let before = font.glyphs[0].clone();
        font.center_glyph_horizontally(0);
        font.center_glyph_vertically(0);
        assert_eq!(font.glyphs[0], before);
    }

    /// Centring must respect column-major pixel indexing.
    #[test]
    fn center_column_major() {
        let mut font = Font::new(8, 8, 16, b'a', 1, true);
        font.set_pixel(0, 7, 0, true);

        font.center_glyph_horizontally(0);
        font.center_glyph_vertically(0);

        assert!(font.get_pixel(0, 3, 3));
        assert!(!font.get_pixel(0, 7, 0));
    }

    #[test]
    fn toggle_pixel() {
        let mut font = Font::new(4, 4, 4, b'a', 1, false);
        assert!(!font.get_pixel(0, 1, 2));
        assert!(font.toggle_pixel(0, 1, 2));
        assert!(font.get_pixel(0, 1, 2));
        assert!(!font.toggle_pixel(0, 1, 2));
        assert!(!font.get_pixel(0, 1, 2));
    }
}
