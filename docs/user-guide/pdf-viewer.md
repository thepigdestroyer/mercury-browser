---
title: PDF viewer
description: Read PDFs in a tab, find text in them, copy from them, and what the viewer cannot do yet.
---

# PDF viewer

A link to a PDF opens it in the tab, in a viewer Mercury draws itself. You scroll, zoom, search and
copy text without leaving the browser, and Back, Forward, Reload and the address bar work as they
do for any page: the tab keeps the PDF's address.

A strip above the pages holds what you need:

- the document's title, taken from the file (or the file's name when it has none);
- the page you are on, with a field to type a page number and go there ("3 / 12");
- **−** and **+** to zoom, with the zoom shown between them;
- **Fit width** and **Fit page**;
- **Rotate**, which turns the view a quarter turn clockwise (the file is not changed);
- **Find**, which opens the find bar;
- **Save a copy** and **Open in Preview**.

On a narrow window the buttons use shorter words (**Width**, **Page**, **Save**, **Preview**).

## Zoom

The viewer's zoom is the tab's page zoom, so everything you already know works: Cmd+= and Cmd+−
zoom in and out, Cmd+0 goes back to your default zoom, and on a trackpad you can pinch. At 100 %
the page is as wide as the window (**Fit width**) or, with **Fit page**, the whole page fits. The
zoom is remembered with the tab, as it is for a web page. When the page is wider than the window
you can scroll sideways, and thin bars at the edge show where you are.

## Keys

| To | Press |
| --- | --- |
| Scroll a screen down or up | Space or Shift+Space, Page Down or Page Up |
| Go to the first or last page | Home or End |
| Scroll a little | the arrow keys (left and right when the page is wider than the window) |
| Search the document | Cmd+F, then Return for the next match, Shift+Return for the previous one |
| Next or previous match | Cmd+G or Shift+Cmd+G |
| Copy the selected text | Cmd+C, or Edit, Copy |
| Clear the selection | Escape |

Use Ctrl instead of Cmd off macOS. The keys work while the keyboard is in the pages; if you are
typing in the address bar, the find bar or the page-number field, they belong to that field.

## Selecting and copying text

Drag across the text to select it. A double click selects a word, a triple click a line, and a
drag can run across pages. Copy puts the text on the clipboard. What you copy is the text as the PDF
stores it, so a column layout may come out in the PDF's own reading order, and some PDFs (scans
without a text layer) have no text to select.

## Finding text

**Find** (or Cmd+F) opens the same find bar as for a web page. Mercury searches the whole document,
page by page, starting at the page you are on. The bar says "3 of 7" while you step through the
matches, and "3 of 7+" while pages are still being searched; matches are marked on the pages, and the
current one is stronger. Closing the bar takes the marks away.

## Links

A link inside the document scrolls to its place. A link to a web address (`http` or `https`) opens
that address in the tab, as a link in a web page does. Other links (an email address, a file, a
script) do nothing: a PDF cannot open files or start programs on your Mac.

## PDFs with a password

If a PDF is protected, the viewer asks for its password before it shows anything. The password goes
only to the part of Mercury that opens the file, and is not saved, logged or kept: if the viewer
has to open the file again later, it asks again. A wrong password says so and lets you try again.

## Where the file goes

Mercury fetches the PDF again for the viewer, with the same cookies as the page, and keeps it in a
private folder inside your Mercury profile. That copy is removed when you close the tab and when
the tab leaves the PDF. If Mercury is ended abruptly, a later start removes copies more than a day old. **Save a copy** saves
the PDF to your Downloads folder like any other download. **Open in Preview** gives the file to the
app your Mac uses for PDFs: a PDF you opened from your own disk is opened where it is, a PDF from
the web is first copied, under its own name, into a private folder of the system's temporary
files, which Mercury clears after a day. A PDF in a window that is run for a screenshot or in the
background never opens another app.

If a site marks a PDF as a file to save (not to show), Mercury saves it and does not open it. A PDF
opened from your own disk (a `file:` address you typed in the address bar, or opened from another
app while Mercury is running) is read where it is and never copied or deleted. A `file:` address
given on the command line when Mercury starts is not counted as typed yet, so its PDF is not shown;
type the address in the address bar instead.

## The setting

**Settings, Downloads, PDF files** chooses what happens when a link leads to a PDF:

- **Open in Mercury Browser** (the default) shows it in the viewer;
- **Open in the system's PDF app** downloads it and opens it in Preview;
- **Download** saves it like any other file.

Without the PDF library (some development builds), PDFs go to the system's PDF app.

## Several PDFs at once

Each open PDF is shown by a separate helper program, so a PDF that crashes the viewer cannot crash
the browser or take another tab with it. To keep memory low, at most four PDFs keep their helper
running: the one in a tab you have not looked at for longest lets go of it, and opens again, at the
same place and zoom, when you come back to the tab. A PDF in a background tab is not opened until
you first show its tab.

If the helper stops (a damaged file can do that), the viewer says "The PDF viewer stopped working
on this file" and offers **Try again**, **Save a copy** and **Open in Preview**. Mercury never
restarts it by itself, so a file that crashes the viewer cannot make it loop.

## What the viewer cannot do yet

- **Forms are shown, not filled.** You can read a form but not type into it, and nothing can be
  saved back into a PDF.
- No **annotations, highlights, signatures** and no printing. Use **Open in Preview** for these.
- No sidebar for **bookmarks or page thumbnails**, no right-click menu, and no Select All.
- **VoiceOver reads the viewer's buttons, the page number and the page you are on, but not the text
  of the pages.** Making page text available to screen readers needs the PDF's structure, which
  the viewer does not read yet. Use **Open in Preview** to read a PDF with VoiceOver for now.
- Pages are always drawn on white, even in dark mode: a PDF is a printed page. Only the area
  around the pages follows your appearance setting.
- Mercury is not (yet) offered as the app your Mac opens PDFs with, and PDFs made by a page
  (`blob:` and `data:` addresses) or sent in answer to a form cannot be opened.
- While you drag a selection, the pages do not scroll by themselves at the edge of the window.

## Safety

A PDF is a program's worth of untrusted input, so Mercury opens it as carefully as it can. The
file is read by PDFium (the PDF library Chrome uses) in a separate helper process that does not
use the network, does not receive your environment variables, and has a time limit on everything
it does.
Documents with more than 20,000 pages, files over 512 MB and absurdly large pictures are refused.
PDFs are drawn without running their scripts, because the PDF library is built without a
JavaScript engine and without form scripting.

One limit to know: the helper does not yet run under an operating-system sandbox (Mercury's page
processes do not either today). If someone found a way to take over the PDF library, the helper
could do what you can do on your Mac. A sandbox for both is planned.

## Trouble

- **"Mercury's PDF viewer is not available in this copy."** The PDF library was not found. Install
  Mercury from a release, or fetch the library as the developer guide says.
- **"This PDF took too long to show and was stopped."** The file is very complex or huge. **Open
  in Preview** may do better.
- **A PDF opens as a download instead.** Check Settings, Downloads, PDF files, and whether the
  site marks the file as an attachment.
