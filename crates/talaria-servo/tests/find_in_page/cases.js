// The find script's cases, run one per Rust test by `find_in_page_script.rs` in boa_engine. Each
// case builds a page in the fake DOM (`fake_dom.js`), runs the script and checks its answer.

defineCase("a page without the text has no matches", () => {
    const document = page(h("p", "hay"));
    assertEqual(find(document, "needle"), "ok;0;-1;0;0");
    assertEqual(document.getSelection().rangeCount, 0);
});

defineCase("one match is selected and scrolled into view", () => {
    const paragraph = h("p", { top: 2000 }, "the needle is here");
    const document = page(paragraph);

    assertEqual(find(document, "needle"), "ok;1;0;0;1");
    assertEqual(selected(document), "needle");
    assertDeepEqual(paragraph.scrolledWith, { block: "center", inline: "nearest" });
});

defineCase("a match already in view is not scrolled", () => {
    const paragraph = h("p", { top: 100 }, "needle");
    const document = page(paragraph);

    find(document, "needle");

    assertEqual(paragraph.scrolledWith, null);
});

defineCase("matching ignores case unless asked", () => {
    const document = page(h("p", "Needle needle NEEDLE nEEdle"));

    assertEqual(find(document, "needle"), "ok;4;0;0;1");
    assertEqual(find(document, "NEEDLE", "here", { matchCase: true }), "ok;1;0;0;1");
    assertEqual(selected(document), "NEEDLE");
});

defineCase("case folding covers non-ASCII letters", () => {
    const document = page(h("p", "ÉCOLE école Straße STRASSE"));

    assertEqual(find(document, "école"), "ok;2;0;0;1");
    assertEqual(find(document, "straße"), "ok;1;0;0;1", "no folding of ß to ss");
});

defineCase("text split across elements is found and selected across them", () => {
    const document = page(h("p", "a nee", h("b", "dl"), h("i", "e"), " b"));

    assertEqual(find(document, "needle"), "ok;1;0;0;1");
    assertEqual(selected(document), "needle");
});

defineCase("matches do not run from one block into the next", () => {
    const document = page(h("p", "foo"), h("p", "bar"), h("div", "foo", h("div", "bar")));

    assertEqual(find(document, "foobar"), "ok;0;-1;0;0");
    assertEqual(find(document, "foo bar"), "ok;0;-1;0;0");
});

defineCase("runs of white space match any white space", () => {
    const document = page(h("p", "hello\n    world"), h("p", "hello", h("br"), "world"));

    assertEqual(find(document, "hello  world"), "ok;2;0;0;1");
});

defineCase("scripts, styles, templates, hidden and unrendered elements are skipped", () => {
    const document = page(
        h("script", "needle"),
        h("style", "needle"),
        h("template", "needle"),
        h("p", { attributes: { hidden: "" } }, "needle"),
        h("p", { rendered: false }, h("span", "needle")),
        h("textarea", "needle"),
    );

    assertEqual(find(document, "needle"), "ok;0;-1;0;0");
});

defineCase("special characters in the query are literal", () => {
    const document = page(h("p", "a.b(c axbxc a.b(c"));

    assertEqual(find(document, "a.b(c"), "ok;2;0;0;1");
    assertEqual(find(document, "[a-z]+"), "ok;0;-1;0;0");
});

defineCase("next and previous move from the selected match and wrap", () => {
    const document = page(h("p", "x1 x2"), h("p", "x3"));

    assertEqual(find(document, "x"), "ok;3;0;0;1");
    assertEqual(find(document, "x", "next"), "ok;3;1;0;1");
    assertEqual(find(document, "x", "next"), "ok;3;2;0;1");
    assertEqual(find(document, "x", "next"), "ok;3;0;0;1");
    assertEqual(find(document, "x", "previous"), "ok;3;2;0;1");
    assertEqual(find(document, "x", "previous"), "ok;3;1;0;1");
    assertEqual(find(document, "x", "here"), "ok;3;1;0;1", "a longer query keeps its place");
});

defineCase("a selection made by the user is where the search starts", () => {
    const second = h("p", "x2");
    const document = page(h("p", "x1"), second, h("p", "x3"));
    const range = document.createRange();
    range.setStart(second.firstChild, 1);
    range.setEnd(second.firstChild, 1);
    document.getSelection().addRange(range);

    assertEqual(find(document, "x", "next"), "ok;3;2;0;1");
});

defineCase("without a selection the reported match is the starting point", () => {
    const document = page(h("p", "x1 x2 x3"));
    const clear = "document.getSelection().removeAllRanges();";
    document.getSelection().removeAllRanges();

    assertEqual(find(document, "x", "next", { current: 1, prepare: clear }), "ok;3;2;0;1");
    document.getSelection().removeAllRanges();
    assertEqual(find(document, "x", "next", { current: 2 }), "ok;3;0;0;1");
    document.getSelection().removeAllRanges();
    assertEqual(find(document, "x", "previous", { current: 0 }), "ok;3;2;0;1");
});

defineCase("a new search with no selection starts at the first match in view", () => {
    const document = page(h("p", { top: -900 }, "x"), h("p", { top: -100 }, "x"), h("p", { top: 300 }, "x"));

    assertEqual(find(document, "x"), "ok;3;2;0;1");
});

defineCase("open shadow roots and slotted content are searched once", () => {
    const light = h("span", "needle light");
    const host = h("div", light);
    host.attachShadow(h("p", "needle shadow"), new FakeSlot([light]));
    const document = page(host);

    assertEqual(find(document, "needle"), "ok;2;0;0;1");
});

defineCase("past the positions it keeps, the search still counts and knows where it is", () => {
    const document = page(h("p", "x ".repeat(1500)));

    assertEqual(find(document, "x"), "ok;1500;0;0;1", "1,000 positions are kept; all are counted");
    assertEqual(find(document, "x", "previous"), "ok;1500;1499;0;1", "the last one is reachable");
});

defineCase("a page that breaks selecting still gets a count", () => {
    const document = page(h("p", "needle needle"));
    const prepare = "Object.getPrototypeOf(document.getSelection()).addRange = function () { throw new Error('no'); };";
    assertEqual(find(document, "needle", "here", { prepare }), "ok;2;0;0;0");
});

defineCase("a page that removes the selection API makes find fail, not throw", () => {
    const document = page(h("p", "needle"));
    document.getSelection = () => null;

    assertEqual(find(document, "needle"), "fail");
});

defineCase("globals the script does not use cannot change the answer", () => {
    const paragraph = h("p", { top: 2000 }, "needle");
    const document = page(paragraph);
    const prepare = `
        globalThis.Range = function () {};
        globalThis.Node = function () {};
        globalThis.Math = { min: () => 99 };
        Object.defineProperty(Object.prototype, "block", { get() { throw new Error("read"); }, configurable: true });
    `;

    assertEqual(find(document, "needle", "here", { prepare }), "ok;1;0;0;1");
    assertDeepEqual(paragraph.scrolledWith, { block: "center", inline: "nearest" });
});

defineCase("a page that replaces RegExp.prototype.exec gets a wrong answer, not an exception", () => {
    const document = page(h("p", "needle"));
    const prepare = "RegExp.prototype.exec = function () { throw new Error('mine'); };";

    assertEqual(find(document, "needle", "here", { prepare }), "fail");
});

defineCase("opening reads a short selection and where it is", () => {
    const paragraph = h("p", "pick the needle here");
    const document = page(paragraph);
    const range = document.createRange();
    range.setStart(paragraph.firstChild, 9);
    range.setEnd(paragraph.firstChild, 15);
    document.getSelection().addRange(range);

    assertEqual(run(document, { mode: "begin", restore: null }), "sel;1.0.0:9|1.0.0:15;needle");
});

defineCase("opening ignores long or multi-line selections and reports none", () => {
    const paragraph = h("p", "line one\nline two");
    const document = page(paragraph);
    assertEqual(run(document, { mode: "begin", restore: null }), "sel;;");

    const range = document.createRange();
    range.setStart(paragraph.firstChild, 0);
    range.setEnd(paragraph.firstChild, 17);
    document.getSelection().addRange(range);
    assertEqual(run(document, { mode: "begin", restore: null }), "sel;1.0.0:0|1.0.0:17;");
});

defineCase("closing puts the page's selection back only while a match is selected", () => {
    const paragraph = h("p", "keep this; find the needle");
    const document = page(paragraph);
    const end = { mode: "end", query: "needle", matchCase: false, restore: "1.0.0:0|1.0.0:4", step: "here", current: -1 };

    find(document, "needle");
    assertEqual(run(document, end), "end;1");
    assertEqual(selected(document), "keep");

    find(document, "needle");
    const elsewhere = document.createRange();
    elsewhere.setStart(paragraph.firstChild, 5);
    elsewhere.setEnd(paragraph.firstChild, 9);
    document.getSelection().removeAllRanges();
    document.getSelection().addRange(elsewhere);
    assertEqual(run(document, end), "end;0");
    assertEqual(selected(document), "this");
});

defineCase("closing rejects a saved position that no longer exists", () => {
    const document = page(h("p", "needle"));
    find(document, "needle");

    assertEqual(
        run(document, { mode: "end", query: "needle", matchCase: false, restore: "9.9.9:0|9.9.9:1", step: "here", current: -1 }),
        "end;0",
    );
    assertEqual(selected(document), "needle");
});

defineCase("several presses at once move that many matches", () => {
    const document = page(h("p", "x1 x2 x3 x4"));

    assertEqual(find(document, "x"), "ok;4;0;0;1");
    assertEqual(find(document, "x", "next", { repeat: 2 }), "ok;4;2;0;1");
    assertEqual(find(document, "x", "previous", { repeat: 3 }), "ok;4;3;0;1");
});

defineCase("previous with no selection goes to the match just above the view", () => {
    const top = h("p", "x2");
    const document = page(h("p", "x1"), top, h("p", "x3"));
    document._atTop = top;

    assertEqual(find(document, "x", "previous"), "ok;3;0;0;1");
    document.getSelection().removeAllRanges();
    assertEqual(find(document, "x"), "ok;3;1;0;1", "a new search starts at the top of the view");
});

defineCase("a match on screen inside an element taller than the view is not scrolled to", () => {
    // A text/plain page: one <pre> 10,000 px tall, scrolled so that 4,000..4,600 is in view.
    const lines = [];
    for (let i = 0; i < 1000; i++) {
        lines.push(i === 410 || i === 900 ? "the needle line" : "filler line");
    }
    const pre = h("pre", { top: -4000, height: 10000 }, lines.join("\n"));
    const document = page(pre);
    const window = fakeWindow();
    const text = pre.firstChild;
    select(document, text, text.data.indexOf("needle"), text.data.indexOf("needle") + 6);

    assertEqual(find(document, "needle", "here", { window }), "ok;2;0;0;1");
    assertEqual(window.scrolledBy, null, "the selected match is already on screen");
    assertEqual(pre.scrolledWith, null, "no jump to the middle of the element");

    assertEqual(find(document, "needle", "next", { window }), "ok;2;1;0;1");
    const offset = text.data.lastIndexOf("needle");
    const expected = -4000 + (offset / text.data.length) * 10000 - 300;
    assertEqual(window.scrolledBy[0], 0);
    assertTrue(Math.abs(window.scrolledBy[1] - expected) < 1, `scrolled by ${window.scrolledBy[1]}, expected ${expected}`);
});

defineCase("the time limit also stops a run of matches that are not rendered", () => {
    // 30,000 matches in an element that is not rendered, each needing no new rendering check.
    const document = page(h("div", { rendered: false }, "x ".repeat(30000)), h("p", "x"));
    let ticks = 0;
    const clock = () => ticks++;

    assertEqual(find(document, "x", "here", { clock }), "ok;0;-1;1;0");
});

defineCase("a long text node that is not rendered is skipped without being read", () => {
    const hidden = "needle ".repeat(20000);
    const document = page(h("div", { rendered: false }, hidden), h("p", "needle"));
    dataReads = [];

    assertEqual(find(document, "needle"), "ok;1;0;0;1");
    assertTrue(!dataReads.includes(hidden.length), "the hidden text was not read");
});

defineCase("the limit cuts the side of the page farthest from the selection", () => {
    const filler = [];
    for (let i = 0; i < 40; i++) {
        filler.push(h("p", "filler ".repeat(8000)));
    }
    const last = h("p", "the last needle");
    const document = page(h("p", "the first needle"), ...filler, last);

    // Typing, with the selection at the end: the end is searched, the far beginning is cut.
    select(document, last.firstChild, 4);
    assertEqual(find(document, "needle"), "ok;1;-1;1;1");
    assertEqual(selected(document), "needle");
    assertEqual(document.getSelection().getRangeAt(0).startContainer, last.firstChild);

    // Return gets a larger allowance and reaches both.
    select(document, last.firstChild, 4);
    assertEqual(find(document, "needle", "here", { thorough: true }), "ok;2;1;0;1");
});

defineCase("a huge text node is read in part around the selection", () => {
    const big = "a".repeat(3000000) + " needle " + "b".repeat(3000000);
    const pre = h("pre", { height: 100000 }, big);
    const document = page(pre);
    select(document, pre.firstChild, 3000001);
    dataReads = [];

    assertEqual(find(document, "needle"), "ok;1;-1;1;1");
    assertEqual(selected(document), "needle");
    assertTrue(!dataReads.includes(big.length), "the node was not read whole");
});

defineCase("text directly inside display: contents counts as rendered", () => {
    const document = page(h("div", h("span", { rendered: false, display: "contents" }, "needle")));

    assertEqual(find(document, "needle"), "ok;1;0;0;1");
});

defineCase("text in the shadow root of a hidden host is not counted", () => {
    const host = h("div", { rendered: false });
    host.attachShadow("needle in a hidden host");
    const shown = h("div");
    shown.attachShadow("needle in a shown host");
    const document = page(host, shown);

    assertEqual(find(document, "needle"), "ok;1;0;0;1");
});

defineCase("an explicit search cut short with nothing found continues from where it stopped next time", () => {
    // 330 paragraphs of 56,000 characters: more than the 16,000,000 one explicit round reads.
    const start = h("p", "start here");
    const body = [start];
    const text = "filler ".repeat(8000);
    for (let i = 0; i < 330; i++) {
        body.push(h("p", text));
    }
    body.push(h("p", "the needle at the very end"));
    const document = page(...body);
    select(document, start.firstChild, 0);
    const press = { thorough: true };

    assertEqual(find(document, "needle", "here", press), "ok;0;-1;1;0", "the first round runs out of room");
    const caret = document.getSelection().getRangeAt(0);
    assertNotEqual(caret.startContainer, start.firstChild, "a caret marks where it stopped");

    assertEqual(find(document, "needle", "here", press), "ok;1;-1;1;1", "the next round goes on from there");
    assertEqual(selected(document), "needle");
});

defineCase("setting up the walk does not run through every sibling of the start", () => {
    const container = h("div");
    for (let i = 0; i < 300000; i++) {
        container.append(h("p", "a"));
    }
    const last = h("p", "the needle");
    container.append(last);
    const document = page(container);
    select(document, last.firstChild, 0);
    calls.nextSibling = 0;

    assertEqual(find(document, "needle"), "ok;1;-1;1;1", "found; the far side is cut");
    assertTrue(calls.nextSibling < 1000, `nextSibling was read ${calls.nextSibling} times`);
});

defineCase("a match that runs across the start point is found", () => {
    const bold = h("b", "dle");
    const document = page(h("p", "a nee", bold, " here"));
    select(document, bold.firstChild, 1);

    assertEqual(find(document, "needle"), "ok;1;0;0;1");
    assertEqual(selected(document), "needle");
});

defineCase("only an explicit search leaves a caret where it stopped", () => {
    const filler = [];
    for (let i = 0; i < 40; i++) {
        filler.push(h("p", "filler ".repeat(8000)));
    }
    const start = h("p", "start here");
    const document = page(start, ...filler, h("p", "the needle at the end"));
    select(document, start.firstChild, 2, 5);

    assertEqual(find(document, "needle"), "ok;0;-1;1;0");
    assertEqual(selected(document), "art", "typing left the page's selection alone");

    assertEqual(find(document, "needle", "here", { thorough: true }).startsWith("ok;"), true);
});

defineCase("closing after a caret puts the page's own selection back", () => {
    const start = h("p", "keep this");
    const filler = [];
    for (let i = 0; i < 40; i++) {
        filler.push(h("p", "filler ".repeat(60000)));
    }
    const document = page(start, ...filler, h("p", "needle"));
    select(document, start.firstChild, 0, 4);
    assertEqual(find(document, "needle", "here", { thorough: true }), "ok;0;-1;1;0");
    assertEqual(selected(document), "", "a caret replaced the selection");

    const end = { mode: "end", query: "needle", matchCase: false, restore: "1.0.0:0|1.0.0:4", caretLeft: true, step: "here", current: -1 };
    assertEqual(run(document, end), "end;1");
    assertEqual(selected(document), "keep");

    select(document, start.firstChild, 0, 0);
    assertEqual(
        run(document, { ...end, restore: null }),
        "end;1",
    );
    assertEqual(document.getSelection().rangeCount, 0, "with nothing to restore, the caret goes");
});

defineCase("in a tall element the place is estimated from the offset in the whole element", () => {
    const pre = h("pre", { top: -4000, height: 10000 }, "a".repeat(5000), "b".repeat(4000) + " needle " + "c".repeat(992));
    const document = page(pre);
    const window = fakeWindow();

    assertEqual(find(document, "needle", "here", { window, thorough: true }), "ok;1;0;0;1");
    const offset = 5000 + 4001;
    const expected = -4000 + (offset / 10000) * 10000 - 300;
    assertTrue(window.scrolledBy !== null && Math.abs(window.scrolledBy[1] - expected) < 1,
        `scrolled by ${window.scrolledBy?.[1]}, expected ${expected}`);
});

defineCase("a page that wraps RegExp and Reflect.apply sees the query, as the header says", () => {
    const document = page(h("p", "needle"));
    const prepare = `
        globalThis.seen = [];
        const OriginalRegExp = RegExp;
        globalThis.RegExp = function (source, flags) {
            seen.push(String(source));
            return new OriginalRegExp(source, flags);
        };
        globalThis.RegExp.prototype = OriginalRegExp.prototype;
        const originalApply = Reflect.apply;
        Reflect.apply = function (target, self, args) {
            seen.push("apply");
            return originalApply(target, self, args);
        };
    `;
    const answer = find(document, "needle", "here", { prepare });

    assertEqual(answer, "ok;1;0;0;1", "pass-through wrappers do not change the answer");
    const seen = globalThis.seen;
    assertTrue(seen.includes("needle"), "the page's RegExp received the query");
    assertTrue(seen.includes("apply"), "the page's Reflect.apply was called");
});

defineCase("rounds make progress inside one long block of inline content", () => {
    // One <div> of 300 inline spans of 56,000 characters: more than one explicit round reads.
    const div = h("div");
    const text = "filler ".repeat(8000);
    for (let i = 0; i < 300; i++) {
        div.append(h("span", text));
    }
    div.append(h("span", "the needle at the very end"));
    const start = h("p", "start here");
    const document = page(start, div);
    select(document, start.firstChild, 0);
    const press = { thorough: true };

    assertEqual(find(document, "needle", "here", press), "ok;0;-1;1;0");
    assertEqual(
        find(document, "needle", "here", press),
        "ok;1;-1;1;1",
        "the second round goes on from the caret, not from the start of the block"
    );
});
