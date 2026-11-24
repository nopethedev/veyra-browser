//! The user-agent stylesheet: the browser's default presentation.

pub const UA_CSS: &str = r#"
html, body, div, p, h1, h2, h3, h4, h5, h6, ul, ol, li, dl, dt, dd, blockquote, pre,
table, form, fieldset, figure, figcaption, header, footer, nav, main, section, article,
aside, address, hr, center, dir, menu, details, summary, dialog, legend, article {
  display: block;
}
head, script, style, title, meta, link, base, template, noscript, datalist, param, source, track {
  display: none;
}
html {
  font-family: DejaVu Sans;
  font-size: 16px;
  color: #000000;
  background-color: #ffffff;
}
body {
  margin: 8px;
  line-height: 1.2;
}
h1 { font-size: 2em; font-weight: bold; margin: 0.67em 0; }
h2 { font-size: 1.5em; font-weight: bold; margin: 0.83em 0; }
h3 { font-size: 1.17em; font-weight: bold; margin: 1em 0; }
h4 { font-size: 1em; font-weight: bold; margin: 1.33em 0; }
h5 { font-size: 0.83em; font-weight: bold; margin: 1.67em 0; }
h6 { font-size: 0.67em; font-weight: bold; margin: 2.33em 0; }
p { margin: 1em 0; }
blockquote { margin: 1em 40px; }
figure { margin: 1em 40px; }
ul, ol, dir, menu { margin: 1em 0; padding-left: 40px; }
ul ul, ul ol, ol ul, ol ol { margin: 0; }
li { display: list-item; }
dl { margin: 1em 0; }
dd { margin-left: 40px; }
pre {
  font-family: DejaVu Sans Mono;
  white-space: pre;
  margin: 1em 0;
  font-size: 0.85em;
}
code, kbd, samp, tt { font-family: DejaVu Sans Mono; font-size: 0.9em; }
a { color: #0000ee; text-decoration: underline; cursor: pointer; }
a:visited { color: #551a8b; }
a:active { color: #ee0000; }
b, strong { font-weight: bold; }
i, em, cite, var, dfn { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
small { font-size: 0.83em; }
big { font-size: 1.17em; }
mark { background-color: #ffff00; color: #000000; }
sup { vertical-align: super; font-size: 0.83em; }
sub { vertical-align: sub; font-size: 0.83em; }
hr {
  display: block;
  border: 1px inset #999999;
  margin: 0.5em auto;
  height: 0px;
}
img { display: inline; }
center { text-align: center; }
table {
  display: table;
  border-collapse: separate;
  border-spacing: 2px;
  border-color: #808080;
}
thead { display: table-header-group; }
tbody { display: table-row-group; }
tfoot { display: table-footer-group; }
tr { display: table-row; }
td, th { display: table-cell; padding: 1px; vertical-align: middle; }
th { font-weight: bold; text-align: center; }
caption { display: table-caption; text-align: center; }
colgroup { display: table-column-group; }
col { display: table-column; }
input, button, select, textarea {
  font-family: DejaVu Sans;
  font-size: 13.333px;
  display: inline-block;
  box-sizing: border-box;
}
input[type="text"], input[type="search"], input[type="email"], input[type="password"],
input[type="number"], input[type="tel"], input[type="url"], input:not([type]) {
  border: 1px solid #767676;
  padding: 1px 2px;
  background-color: #ffffff;
  width: 170px;
  height: 21px;
  cursor: text;
}
textarea {
  border: 1px solid #767676;
  padding: 1px 2px;
  background-color: #ffffff;
  width: 170px;
  height: 51px;
  display: inline-block;
  white-space: pre-wrap;
  cursor: text;
}
button {
  border: 1px solid #767676;
  padding: 1px 6px;
  background-color: #efefef;
  text-align: center;
  cursor: pointer;
  min-height: 21px;
}
input[type="submit"], input[type="button"], input[type="reset"] {
  border: 1px solid #767676;
  padding: 1px 6px;
  background-color: #efefef;
  text-align: center;
  width: auto;
  height: auto;
  cursor: pointer;
}
input[type="checkbox"], input[type="radio"] {
  width: 13px;
  height: 13px;
  margin: 3px;
  cursor: pointer;
  border: 1px solid #767676;
  background-color: #ffffff;
}
input[type="hidden"] { display: none; }
input[type="range"] { width: 129px; height: 16px; }
input[type="file"] { width: auto; border: none; background: transparent; }
select {
  border: 1px solid #767676;
  background-color: #ffffff;
  padding: 1px;
  min-width: 60px;
  cursor: pointer;
}
option { display: block; padding: 0 2px; }
fieldset {
  border: 1px solid #808080;
  margin: 0 2px;
  padding: 0.35em 0.75em 0.625em;
}
legend { padding: 0 2px; }
details > summary { display: block; cursor: pointer; }
dialog {
  position: absolute;
  left: 0; right: 0;
  margin: auto;
  border: 1px solid #000;
  padding: 1em;
  background: #fff;
}
dialog:not([open]) { display: none; }
[hidden] { display: none !important; }
q::before { content: "\0022"; }
q::after { content: "\0022"; }
iframe { display: inline-block; width: 300px; height: 150px; border: 2px inset #999; }
canvas, video { display: inline-block; }
br { display: inline; }
"#;
