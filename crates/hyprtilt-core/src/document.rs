// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! What a configuration backend reports about a file, and the edits it
//! produces. Both backends (`lua` and `hyprlang`) return these types, so
//! the rest of hyprtilt does not care which language a file is in.

use std::ops::Range;

use crate::block::BlockError;
use crate::model::MonitorRule;

/// The configuration language of a target file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Lua configuration (`hl.monitor({...})`), Hyprland 0.55 and later.
    Lua,
    /// hyprlang configuration (`monitor=...`, `monitorv2 {}`).
    Hyprlang,
}

impl Backend {
    /// The backend Hyprland would use for a file name: `.lua` is Lua,
    /// anything else is hyprlang.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::document::Backend;
    /// use std::path::Path;
    ///
    /// assert_eq!(Backend::for_path(Path::new("hypr-user.lua")), Backend::Lua);
    /// assert_eq!(Backend::for_path(Path::new("hyprland.conf")), Backend::Hyprlang);
    /// ```
    #[must_use]
    pub fn for_path(path: &std::path::Path) -> Backend {
        if path.extension().is_some_and(|e| e == "lua") {
            Backend::Lua
        } else {
            Backend::Hyprlang
        }
    }
}

impl Backend {
    /// The block markers of the language.
    #[must_use]
    pub fn markers(self) -> &'static crate::block::Markers {
        match self {
            Backend::Lua => &crate::block::LUA_MARKERS,
            Backend::Hyprlang => &crate::block::HYPRLANG_MARKERS,
        }
    }

    /// Read the managed block and the rules outside it.
    ///
    /// # Errors
    ///
    /// See [`crate::lua::parse`] and [`crate::hyprlang::parse`].
    pub fn parse(self, src: &str) -> Result<ConfigDocument, ConfigError> {
        match self {
            Backend::Lua => crate::lua::parse(src),
            Backend::Hyprlang => crate::hyprlang::parse(src),
        }
    }

    /// Write `rules` into the managed block.
    ///
    /// # Errors
    ///
    /// See [`crate::lua::save`] and [`crate::hyprlang::save`].
    pub fn save(
        self,
        src: &str,
        rules: &[MonitorRule],
        options: &SaveOptions,
    ) -> Result<Edit, ConfigError> {
        match self {
            Backend::Lua => crate::lua::save(src, rules),
            Backend::Hyprlang => crate::hyprlang::save(src, rules, options),
        }
    }

    /// Move rules from outside the block into it.
    ///
    /// # Errors
    ///
    /// See [`crate::lua::adopt`] and [`crate::hyprlang::adopt`].
    pub fn adopt(self, src: &str, lines: &[usize]) -> Result<Edit, ConfigError> {
        match self {
            Backend::Lua => crate::lua::adopt(src, lines),
            Backend::Hyprlang => crate::hyprlang::adopt(src, lines),
        }
    }

    /// Remove the block markers.
    ///
    /// # Errors
    ///
    /// See [`crate::lua::unmanage`] and [`crate::hyprlang::unmanage`].
    pub fn unmanage(self, src: &str) -> Result<Edit, ConfigError> {
        match self {
            Backend::Lua => crate::lua::unmanage(src),
            Backend::Hyprlang => crate::hyprlang::unmanage(src),
        }
    }

    /// The text of one rule in the language.
    ///
    /// # Errors
    ///
    /// See [`crate::lua::format_rule`] and [`crate::hyprlang::format_rule`].
    pub fn format_rule(
        self,
        rule: &MonitorRule,
        options: &SaveOptions,
    ) -> Result<String, ConfigError> {
        match self {
            Backend::Lua => crate::lua::format_rule(rule),
            Backend::Hyprlang => crate::hyprlang::format_rule(rule, options),
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Backend::Lua => "lua",
            Backend::Hyprlang => "hyprlang",
        })
    }
}

/// A monitor rule found outside the managed block.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundRule {
    /// The rule, if every value is a literal hyprtilt understands. `None`
    /// for rules built from variables, function calls or `$vars`.
    pub rule: Option<MonitorRule>,
    /// The source text of the whole rule statement.
    pub text: String,
    /// Byte range of the statement, including a trailing `;` and the rest
    /// of its line when that is only whitespace or a comment. Removing this
    /// range removes the rule cleanly.
    pub span: Range<usize>,
    /// 1-based line where the statement starts.
    pub line: usize,
    /// Why the rule cannot be adopted, if it cannot: not literal, inside a
    /// function or conditional, several statements on its line, ...
    pub not_adoptable: Option<String>,
}

impl FoundRule {
    /// Whether `adopt` can move this rule into the block.
    #[must_use]
    pub fn is_adoptable(&self) -> bool {
        self.rule.is_some() && self.not_adoptable.is_none()
    }
}

/// The managed block of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct ManagedBlock {
    /// Byte range of the whole block, marker lines included.
    pub span: Range<usize>,
    /// 1-based line of the begin marker.
    pub begin_line: usize,
    /// 1-based line of the end marker.
    pub end_line: usize,
    /// The rules in the block, in order.
    pub rules: Vec<MonitorRule>,
    /// 1-based line where each rule starts, parallel to `rules`.
    pub rule_lines: Vec<usize>,
}

/// Everything a backend knows about a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfigDocument {
    /// The managed block, if the file has one.
    pub block: Option<ManagedBlock>,
    /// Monitor rules outside the block, in file order.
    pub outside: Vec<FoundRule>,
}

impl ConfigDocument {
    /// The rules in the block, or none.
    #[must_use]
    pub fn block_rules(&self) -> &[MonitorRule] {
        self.block.as_ref().map_or(&[], |b| b.rules.as_slice())
    }

    /// Rules outside the block whose selector string equals one of the
    /// block's selectors. In Lua they merge with the block's rules, in
    /// hyprlang they are replaced by whichever comes later; either way the
    /// user should know about them.
    #[must_use]
    pub fn conflicting_outside(&self) -> Vec<&FoundRule> {
        let selectors: Vec<&str> = self
            .block_rules()
            .iter()
            .map(|r| r.output.as_str())
            .collect();
        self.outside
            .iter()
            .filter(|f| {
                f.rule
                    .as_ref()
                    .is_some_and(|r| selectors.contains(&r.output.as_str()))
            })
            .collect()
    }
}

/// The result of an edit: the new file content and whether anything changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The complete new content of the file.
    pub content: String,
    /// `false` when `content` equals the input byte for byte; the file must
    /// then not be written at all (no backup, no reload).
    pub changed: bool,
}

impl Edit {
    /// An edit from the old and new content.
    #[must_use]
    pub fn new(old: &str, content: String) -> Edit {
        let changed = old != content;
        Edit { content, changed }
    }
}

/// Why a file cannot be read or edited.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ConfigError {
    /// The markers are malformed.
    #[error(transparent)]
    Block(#[from] BlockError),
    /// The file is not valid in its language, as far as hyprtilt can tell.
    #[error("line {line}: {message}")]
    Syntax {
        /// 1-based line.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// The managed block contains something other than literal rules,
    /// comments and blank lines; hyprtilt refuses to rewrite it.
    #[error("line {line}: the managed block contains content hyprtilt cannot rewrite: {message}")]
    UnsupportedInBlock {
        /// 1-based line.
        line: usize,
        /// What was found.
        message: String,
    },
    /// A rule cannot be expressed in the target language or version.
    #[error("rule for {output:?}: {message}")]
    Unrepresentable {
        /// The rule's selector.
        output: String,
        /// Why.
        message: String,
    },
    /// `adopt` was asked for a rule that cannot be adopted.
    #[error("line {line}: rule cannot be adopted: {reason}")]
    NotAdoptable {
        /// 1-based line of the rule.
        line: usize,
        /// Why.
        reason: String,
    },
    /// `adopt` would move rules past a rule that stays outside the block
    /// and may concern the same monitor, which could change which rule
    /// wins.
    #[error(
        "line {line}: this monitor rule stays outside the block but sits between rules \
         being adopted, and moving them past it could change which rule wins; adopt the \
         rules on each side of it separately, or move it by hand"
    )]
    AdoptCrossing {
        /// 1-based line of the rule in the way.
        line: usize,
    },
    /// `unmanage` on a file without a block.
    #[error("the file has no managed block")]
    NoBlock,
}

/// What the backends need to know about the target Hyprland when writing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveOptions {
    /// The running Hyprland version, if known. hyprlang fields that need a
    /// newer version are refused; unknown means everything is allowed.
    pub hyprland: Option<crate::version::Version>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn found(output: &str, adoptable: bool) -> FoundRule {
        FoundRule {
            rule: Some(MonitorRule::new(output)),
            text: String::new(),
            span: 0..0,
            line: 1,
            not_adoptable: (!adoptable).then(|| "inside a function".to_owned()),
        }
    }

    #[test]
    fn backend_from_extension() {
        assert_eq!(Backend::for_path(Path::new("/a/b.lua")), Backend::Lua);
        assert_eq!(Backend::for_path(Path::new("/a/b.LUA")), Backend::Hyprlang);
        assert_eq!(Backend::for_path(Path::new("/a/b")), Backend::Hyprlang);
        assert_eq!(Backend::Lua.to_string(), "lua");
        assert_eq!(Backend::Hyprlang.to_string(), "hyprlang");
    }

    #[test]
    fn conflicts_are_by_exact_selector() {
        let doc = ConfigDocument {
            block: Some(ManagedBlock {
                span: 0..0,
                begin_line: 1,
                end_line: 2,
                rules: vec![MonitorRule::new("DP-1")],
                rule_lines: vec![2],
            }),
            outside: vec![
                found("DP-1", true),
                found("desc:Samsung", true),
                found("", false),
            ],
        };
        let c = doc.conflicting_outside();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].rule.as_ref().unwrap().output.as_str(), "DP-1");
        assert!(doc.outside[0].is_adoptable());
        assert!(!doc.outside[2].is_adoptable());
        assert!(ConfigDocument::default().block_rules().is_empty());
    }

    #[test]
    fn dispatch_by_backend() {
        let lua = "hl.monitor({ output = \"DP-1\" })\n";
        let conf = "monitor = DP-1, preferred, auto, 1\n";
        let opts = SaveOptions::default();
        for (backend, src) in [(Backend::Lua, lua), (Backend::Hyprlang, conf)] {
            let doc = backend.parse(src).unwrap();
            assert_eq!(doc.outside.len(), 1);
            let adopted = backend.adopt(src, &[]).unwrap().content;
            assert!(adopted.contains(backend.markers().begin));
            let rules = backend.parse(&adopted).unwrap().block_rules().to_vec();
            assert!(!backend.save(&adopted, &rules, &opts).unwrap().changed);
            assert_eq!(backend.unmanage(&adopted).unwrap().content, src);
            assert!(
                backend
                    .format_rule(&rules[0], &opts)
                    .unwrap()
                    .contains("DP-1")
            );
        }
    }

    #[test]
    fn edit_detects_changes() {
        assert!(!Edit::new("a", "a".to_owned()).changed);
        assert!(Edit::new("a", "b".to_owned()).changed);
    }

    #[test]
    fn error_messages() {
        let e = ConfigError::UnsupportedInBlock {
            line: 7,
            message: "local x = 1".to_owned(),
        };
        assert_eq!(
            e.to_string(),
            "line 7: the managed block contains content hyprtilt cannot rewrite: local x = 1"
        );
        let e: ConfigError = BlockError::Unclosed { line: 3 }.into();
        assert_eq!(
            e.to_string(),
            "line 3: managed block is not closed (no end marker)"
        );
    }
}
