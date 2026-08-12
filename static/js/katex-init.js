// Render math once KaTeX and its auto-render extension have loaded.
//
// This was an `onload="renderMathInElement(...)"` attribute on the auto-render script
// tag. Inline event handlers are blocked by a script-src without 'unsafe-inline', so it
// moved here. Load this with `defer` *after* both KaTeX files: deferred scripts execute
// in document order, once parsing is done and before DOMContentLoaded, so document.body
// is complete and renderMathInElement is already defined.
renderMathInElement(document.body, {
    delimiters: [
        { left: '$$', right: '$$', display: true },
        { left: '$', right: '$', display: false }
    ],
    throwOnError: false
});
