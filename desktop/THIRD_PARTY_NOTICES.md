# Third-party material

## PdfCraft

Parts of the in-place Edit Text display (`crates/pagify_app/src/in_place.rs`) are adapted
from PdfCraft, https://github.com/storytold/pdfcraft (`crates/ui-egui/src/edit_text_ui.rs`,
revision `0dc77fe`). PdfCraft is available under the MIT License or the Apache License 2.0;
Pagify uses it under the MIT License:

```
MIT License

Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors

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

The ArtCraft name, wordmark and logos are trademarks of the ArtCraft Team and are not used here.

## Montserrat

`third_party/fonts/Montserrat-{Light,Regular,Bold,ExtraBold,Black}.ttf` are static
instances of Montserrat, the typeface by Julieta Ulanovsky and the Montserrat
Project Authors (https://github.com/JulietaUla/Montserrat), taken from Google
Fonts (https://github.com/google/fonts/tree/main/ofl/montserrat). They are used
for outlined-text recognition and are embedded into saved documents when a page
names a Montserrat face the file does not carry.

Montserrat is licensed under the SIL Open Font License, Version 1.1. The licence
text is `third_party/fonts/LICENSE.txt`. The fonts may be bundled and
redistributed with this application; they may not be sold on their own, and a
modified version may not use the reserved font name.

## PT Sans

`third_party/fonts/PTSans-Regular.ttf` is PT Sans Regular by ParaType Ltd.
(version 2.003W), taken unmodified from Google Fonts
(https://github.com/google/fonts/tree/main/ofl/ptsans). It is embedded into
saved documents when a page names `PTSans-Regular` without carrying it.

PT Sans is licensed under the SIL Open Font License, Version 1.1, with the
Reserved Font Names "PT Sans" and "ParaType". The licence text is
`third_party/fonts/PTSans-OFL.txt`. The font is bundled unmodified; it may not
be sold on its own, and a modified version may not use the reserved names.
