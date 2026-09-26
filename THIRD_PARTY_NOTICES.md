# Third-party notices

CSV Columns is MIT licensed (see `LICENSE`). It includes or derives from the
following third-party material.

## zed-rainbow-csv

<https://github.com/Kalmaegi/zed-rainbow-csv>

Used for: `languages/*/config.toml` and `languages/*/highlights.scm` (display
names changed), and the small sample files in `samples/`.

```text
MIT License

Copyright (c) 2024 Hans

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## vscode_rainbow_csv

<https://github.com/mechatroner/vscode_rainbow_csv>

Used for: the Align/Shrink/virtual-align logic in `crates/rainbow_csv_core`,
reimplemented in Rust from `rbql_core/rbql-js/csv_utils.js` and
`rainbow_utils.js`. No source files are included.

```text
MIT License

Copyright (c) 2017 Dmitry Ignatovich

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## USGS earthquake catalog sample

`samples/usgs_earthquakes_2025h1.csv`: earthquakes located by the USGS
National Earthquake Information Center (`catalog=us`), 2025-01-01 to
2025-07-01, retrieved on 2026-09-26 from:

<https://earthquake.usgs.gov/fdsnws/event/1/query?format=csv&catalog=us&starttime=2025-01-01&endtime=2025-07-01&orderby=time-asc>

USGS-authored data is in the U.S. public domain. Credit: U.S. Geological
Survey, Earthquake Hazards Program.
