; Headings as definitions, for the outline panel.
;
; `tree-sitter-md` ships no tags query of its own — every other grammar the outline uses carries
; one, so this is the single vendored gap-filler. It runs against the *block* grammar, where a
; heading's text is the `heading_content` field. The ATX level (`atx_h1_marker` … `atx_h6_marker`)
; is deliberately not captured: `Def` is flat, so a level would have nowhere to go.

(atx_heading
  heading_content: (inline) @name) @definition.heading

(setext_heading
  heading_content: (paragraph) @name) @definition.heading
