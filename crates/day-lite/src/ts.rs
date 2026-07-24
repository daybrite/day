//! TypeScript → JavaScript at module-load time (docs/lite.md §1): the oxc pipeline strips
//! types (and compiles TS-only syntax like enums) so `.ts` miniapp sources run directly.
//! No decorators, no JSX — those are load errors, reported with the file name.

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

/// Strip types from `source` (a module). `name` is used in diagnostics only.
pub fn strip(name: &str, source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::ts().with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        let first = parsed
            .diagnostics
            .iter()
            .map(|d| d.to_string())
            .next()
            .unwrap_or_default();
        return Err(format!("{name}: {first}"));
    }
    let mut program = parsed.program;
    let scoping = oxc_semantic::SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let ret = Transformer::new(
        &allocator,
        std::path::Path::new(name),
        &TransformOptions::default(),
    )
    .build_with_scoping(scoping, &mut program);
    if !ret.diagnostics.is_empty() {
        let first = ret
            .diagnostics
            .iter()
            .map(|d| d.to_string())
            .next()
            .unwrap_or_default();
        return Err(format!("{name}: {first}"));
    }
    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_interfaces_and_annotations() {
        let js = super::strip(
            "t.ts",
            "interface A { n: number }\nexport const f = (a: A): number => a.n * 2;",
        )
        .expect("strip");
        assert!(!js.contains("interface"));
        assert!(!js.contains(": number"));
        assert!(js.contains("a.n * 2"));
    }

    #[test]
    fn reports_syntax_errors_with_the_file_name() {
        let err = super::strip("bad.ts", "const = ;").unwrap_err();
        assert!(err.starts_with("bad.ts:"));
    }
}
