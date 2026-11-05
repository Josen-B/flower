//! Shared deterministic 96-DPI width estimates, not platform/font measurements.
pub const DEFAULT_CANDIDATE_WIDTH: u32 = 544;
pub const MAX_CANDIDATE_CELL_WIDTH: u32 = 240;

pub fn candidate_cell_width(text: &str, index: usize) -> u32 {
    let label = ((index + 1).to_string().len() as u32 + 2).saturating_mul(9);
    let text_width = text.chars().take(240).fold(0u32, |width, c| {
        width.saturating_add(if c.is_ascii() { 9 } else { 18 })
    });
    label
        .saturating_add(text_width)
        .saturating_add(16)
        .clamp(48, MAX_CANDIDATE_CELL_WIDTH)
}
