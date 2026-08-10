use skiff_artifact_model::{InstructionSourceSite, SourcePosition, SourceSpanRef};

use crate::shared::error::SourceSpan;

const FILE_IR_UNIT_SOURCE_ID: u64 = 0;

pub(super) fn source_instruction_site(span: SourceSpan) -> Result<InstructionSourceSite, String> {
    Ok(InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: FILE_IR_UNIT_SOURCE_ID,
            start: source_position(span.start.line, span.start.column, span.start.offset)?,
            end: source_position(span.end.line, span.end.column, span.end.offset)?,
        },
    })
}

fn source_position(line: usize, column: usize, offset: usize) -> Result<SourcePosition, String> {
    Ok(SourcePosition {
        line: u32::try_from(line)
            .map_err(|_| format!("source line {line} exceeds the source-site u32 range"))?,
        column: u32::try_from(column)
            .map_err(|_| format!("source column {column} exceeds the source-site u32 range"))?,
        offset: Some(
            u32::try_from(offset)
                .map_err(|_| format!("source offset {offset} exceeds the source-site u32 range"))?,
        ),
    })
}
