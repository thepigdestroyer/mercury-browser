// Tests for `src/find_in_page.js` against a small fake DOM, run with `node --test`. The
// `find_in_page_script` Rust test runs this file when Node.js is installed. The real engine is
// covered by the `--find` runs against `tests/pages/find-*.html` (see the find report).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const script = readFileSync(new URL("../src/find_in_page.js", import.meta.url), "utf8");

class FakeNode {
    constructor(type) {
        this._type = type;
        this._children = [];
        this._parent = null;
    }
    get nodeType() {
        return this._type;
    }
    get firstChild() {
        return this._children[0] ?? null;
    }
    get lastChild() {
        return this._children[this._children.length - 1] ?? null;
    }
    get nextSibling() {
        calls.nextSibling++;
        return this._parent === null ? null : this._parent._children[this._index + 1] ?? null;
    }
    get previousSibling() {
        calls.previousSibling++;
        return this._parent === null ? null : this._parent._children[this._index - 1] ?? null;
    }
    get parentNode() {
        return this._parent;
    }
    append(...children) {
        for (const child of children) {
            const node = typeof child === "string" ? new FakeText(child) : child;
            node._parent = this;
            node._index = this._children.length;
            this._children.push(node);
        }
        return this;
    }
}

class FakeCharacterData extends FakeNode {
    constructor(data) {
        super(3);
        this._data = data;
    }
    get data() {
        dataReads.push(this._data.length);
        return this._data;
    }
    get length() {
        return this._data.length;
    }
    substringData(offset, count) {
        return this._data.slice(offset, offset + count);
    }
}

// The lengths of the text nodes whose whole `data` the script read.
let dataReads = [];
// How often the sibling accessors were called.
const calls = { nextSibling: 0, previousSibling: 0 };

class FakeText extends FakeCharacterData {}

class FakeElement extends FakeNode {
    constructor(name, options = {}) {
        super(1);
        this._name = name;
        this._attributes = options.attributes ?? {};
        this._rendered = options.rendered ?? true;
        this._top = options.top ?? 0;
        this._height = options.height ?? 20;
        this._display = options.display ?? "block";
        this._shadow = null;
        this.scrolledWith = null;
    }
    get localName() {
        return this._name;
    }
    hasAttribute(name) {
        return Object.hasOwn(this._attributes, name);
    }
    getClientRects() {
        for (let element = this; element !== null && element._type === 1; element = element._parent) {
            if (!element._rendered) {
                return { length: 0 };
            }
        }
        return { length: 1 };
    }
    getBoundingClientRect() {
        return { top: this._top, bottom: this._top + this._height, height: this._height, left: 0, right: 100 };
    }
    scrollIntoView(options) {
        this.scrolledWith = { block: options.block, inline: options.inline };
    }
    get clientHeight() {
        return 600;
    }
    get clientWidth() {
        return 800;
    }
    get shadowRoot() {
        return this._shadow;
    }
    attachShadow(...children) {
        this._shadow = new FakeShadowRoot(this).append(...children);
        return this;
    }
}

class FakeShadowRoot extends FakeNode {
    constructor(host) {
        super(11);
        this._host = host;
    }
    get host() {
        return this._host;
    }
}

class FakeSlot extends FakeElement {
    constructor(assigned) {
        super("slot");
        this._assigned = assigned;
    }
    assignedNodes() {
        return this._assigned;
    }
}

class FakeRange {
    setStart(node, offset) {
        checkOffset(node, offset);
        this._startContainer = node;
        this._startOffset = offset;
    }
    setEnd(node, offset) {
        checkOffset(node, offset);
        this._endContainer = node;
        this._endOffset = offset;
    }
    get startContainer() {
        return this._startContainer;
    }
    get startOffset() {
        return this._startOffset;
    }
    get endContainer() {
        return this._endContainer;
    }
    get endOffset() {
        return this._endOffset;
    }
}

function checkOffset(node, offset) {
    const length = node._type === 3 ? node._data.length : node._children.length;
    if (offset > length) {
        throw new Error("IndexSizeError");
    }
}

class FakeSelection {
    constructor(document) {
        this._document = document;
        this._ranges = [];
    }
    get rangeCount() {
        return this._ranges.length;
    }
    getRangeAt(index) {
        return this._ranges[index];
    }
    removeAllRanges() {
        this._ranges = [];
    }
    addRange(range) {
        this._ranges.push(range);
    }
    toString() {
        const range = this._ranges[0];
        if (range === undefined) {
            return "";
        }
        let text = "";
        let inside = false;
        for (const node of textNodes(this._document)) {
            const start = node === range.startContainer ? range.startOffset : 0;
            if (node === range.startContainer) {
                inside = true;
            }
            if (!inside) {
                continue;
            }
            if (node === range.endContainer && node === range.startContainer && range.startOffset === range.endOffset) {
            return "";
        }
        if (node === range.endContainer) {
                return text + node._data.slice(start, range.endOffset);
            }
            text += node._data.slice(start);
        }
        return text;
    }
}

function textNodes(node, found = []) {
    if (node._type === 3) {
        found.push(node);
    }
    for (const child of node._shadow ? node._shadow._children : node._children) {
        textNodes(child, found);
    }
    return found;
}

class FakeDocument extends FakeNode {
    constructor() {
        super(9);
        this._selection = new FakeSelection(this);
    }
    get documentElement() {
        return this._children[0] ?? null;
    }
    createRange() {
        return new FakeRange();
    }
    createTextNode(data) {
        return new FakeText(data);
    }
    getSelection() {
        return this._selection;
    }
    elementFromPoint() {
        return this._atTop ?? null;
    }
}

function fakeWindow() {
    return {
        scrolledBy: null,
        getComputedStyle(element) {
            return { getPropertyValue: (name) => (name === "display" ? element._display : "") };
        },
        scrollBy(x, y) {
            this.scrolledBy = [x, y];
        },
    };
}

function h(name, options, ...children) {
    if (typeof options === "string" || options instanceof FakeNode) {
        children.unshift(options);
        options = {};
    }
    return new FakeElement(name, options ?? {}).append(...children);
}

function page(...bodyChildren) {
    const document = new FakeDocument();
    document.append(h("html", {}, h("head", {}, h("title", {}, "needle")), h("body", {}, ...bodyChildren)));
    return document;
}

function run(document, request, prepare, clock = () => 0, window = fakeWindow()) {
    const context = vm.createContext({ document, window, performance: { now: clock } });
    if (prepare !== undefined) {
        vm.runInContext(prepare, context);
    }
    const before = Object.getOwnPropertyNames(context).sort();
    const answer = vm.runInContext(`(${script})(${JSON.stringify(request)})`, context);
    assert.deepEqual(Object.getOwnPropertyNames(context).sort(), before, "no globals are left behind");
    return answer;
}

function find(document, query, step = "here", options = {}) {
    return run(document, {
        mode: "find",
        query,
        matchCase: options.matchCase ?? false,
        step,
        repeat: options.repeat ?? 1,
        thorough: options.thorough ?? false,
        current: options.current ?? -1,
        restore: null,
    }, options.prepare, options.clock, options.window);
}

function select(document, node, start, end = start) {
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, end);
    document.getSelection().removeAllRanges();
    document.getSelection().addRange(range);
}

function selected(document) {
    return document.getSelection().toString();
}

test("a page without the text has no matches", () => {
    const document = page(h("p", "hay"));
    assert.equal(find(document, "needle"), "ok;0;-1;0;0");
    assert.equal(document.getSelection().rangeCount, 0);
});

test("one match is selected and scrolled into view", () => {
    const paragraph = h("p", { top: 2000 }, "the needle is here");
    const document = page(paragraph);

    assert.equal(find(document, "needle"), "ok;1;0;0;1");
    assert.equal(selected(document), "needle");
    assert.deepEqual(paragraph.scrolledWith, { block: "center", inline: "nearest" });
});

test("a match already in view is not scrolled", () => {
    const paragraph = h("p", { top: 100 }, "needle");
    const document = page(paragraph);

    find(document, "needle");

    assert.equal(paragraph.scrolledWith, null);
});

test("matching ignores case unless asked", () => {
    const document = page(h("p", "Needle needle NEEDLE nEEdle"));

    assert.equal(find(document, "needle"), "ok;4;0;0;1");
    assert.equal(find(document, "NEEDLE", "here", { matchCase: true }), "ok;1;0;0;1");
    assert.equal(selected(document), "NEEDLE");
});

test("case folding covers non-ASCII letters", () => {
    const document = page(h("p", "ÉCOLE école Straße STRASSE"));

    assert.equal(find(document, "école"), "ok;2;0;0;1");
    assert.equal(find(document, "straße"), "ok;1;0;0;1", "no folding of ß to ss");
});

test("text split across elements is found and selected across them", () => {
    const document = page(h("p", "a nee", h("b", "dl"), h("i", "e"), " b"));

    assert.equal(find(document, "needle"), "ok;1;0;0;1");
    assert.equal(selected(document), "needle");
});

test("matches do not run from one block into the next", () => {
    const document = page(h("p", "foo"), h("p", "bar"), h("div", "foo", h("div", "bar")));

    assert.equal(find(document, "foobar"), "ok;0;-1;0;0");
    assert.equal(find(document, "foo bar"), "ok;0;-1;0;0");
});

test("runs of white space match any white space", () => {
    const document = page(h("p", "hello\n    world"), h("p", "hello", h("br"), "world"));

    assert.equal(find(document, "hello  world"), "ok;2;0;0;1");
});

test("scripts, styles, templates, hidden and unrendered elements are skipped", () => {
    const document = page(
        h("script", "needle"),
        h("style", "needle"),
        h("template", "needle"),
        h("p", { attributes: { hidden: "" } }, "needle"),
        h("p", { rendered: false }, h("span", "needle")),
        h("textarea", "needle"),
    );

    assert.equal(find(document, "needle"), "ok;0;-1;0;0");
});

test("special characters in the query are literal", () => {
    const document = page(h("p", "a.b(c axbxc a.b(c"));

    assert.equal(find(document, "a.b(c"), "ok;2;0;0;1");
    assert.equal(find(document, "[a-z]+"), "ok;0;-1;0;0");
});

test("next and previous move from the selected match and wrap", () => {
    const document = page(h("p", "x1 x2"), h("p", "x3"));

    assert.equal(find(document, "x"), "ok;3;0;0;1");
    assert.equal(find(document, "x", "next"), "ok;3;1;0;1");
    assert.equal(find(document, "x", "next"), "ok;3;2;0;1");
    assert.equal(find(document, "x", "next"), "ok;3;0;0;1");
    assert.equal(find(document, "x", "previous"), "ok;3;2;0;1");
    assert.equal(find(document, "x", "previous"), "ok;3;1;0;1");
    assert.equal(find(document, "x", "here"), "ok;3;1;0;1", "a longer query keeps its place");
});

test("a selection made by the user is where the search starts", () => {
    const second = h("p", "x2");
    const document = page(h("p", "x1"), second, h("p", "x3"));
    const range = document.createRange();
    range.setStart(second.firstChild, 1);
    range.setEnd(second.firstChild, 1);
    document.getSelection().addRange(range);

    assert.equal(find(document, "x", "next"), "ok;3;2;0;1");
});

test("without a selection the reported match is the starting point", () => {
    const document = page(h("p", "x1 x2 x3"));
    const clear = "document.getSelection().removeAllRanges();";
    document.getSelection().removeAllRanges();

    assert.equal(find(document, "x", "next", { current: 1, prepare: clear }), "ok;3;2;0;1");
    document.getSelection().removeAllRanges();
    assert.equal(find(document, "x", "next", { current: 2 }), "ok;3;0;0;1");
    document.getSelection().removeAllRanges();
    assert.equal(find(document, "x", "previous", { current: 0 }), "ok;3;2;0;1");
});

test("a new search with no selection starts at the first match in view", () => {
    const document = page(h("p", { top: -900 }, "x"), h("p", { top: -100 }, "x"), h("p", { top: 300 }, "x"));

    assert.equal(find(document, "x"), "ok;3;2;0;1");
});

test("open shadow roots and slotted content are searched once", () => {
    const light = h("span", "needle light");
    const host = h("div", light);
    host.attachShadow(h("p", "needle shadow"), new FakeSlot([light]));
    const document = page(host);

    assert.equal(find(document, "needle"), "ok;2;0;0;1");
});

test("past the positions it keeps, the search still counts and knows where it is", () => {
    const document = page(h("p", "x ".repeat(1500)));

    assert.equal(find(document, "x"), "ok;1500;0;0;1", "1,000 positions are kept; all are counted");
    assert.equal(find(document, "x", "previous"), "ok;1500;1499;0;1", "the last one is reachable");
});

test("a page that breaks selecting still gets a count", () => {
    const document = page(h("p", "needle needle"));
    const prepare = "Object.getPrototypeOf(document.getSelection()).addRange = function () { throw new Error('no'); };";
    try {
        assert.equal(
            find(document, "needle", "here", { prepare }),
            "ok;2;0;0;0",
        );
    } finally {
        FakeSelection.prototype.addRange = function (range) {
            this._ranges.push(range);
        };
    }
});

test("a page that removes the selection API makes find fail, not throw", () => {
    const document = page(h("p", "needle"));
    document.getSelection = () => null;

    assert.equal(find(document, "needle"), "fail");
});

test("globals the script does not use cannot change the answer", () => {
    const paragraph = h("p", { top: 2000 }, "needle");
    const document = page(paragraph);
    const prepare = `
        globalThis.Range = function () {};
        globalThis.Node = function () {};
        globalThis.Math = { min: () => 99 };
        Object.defineProperty(Object.prototype, "block", { get() { throw new Error("read"); }, configurable: true });
    `;

    assert.equal(find(document, "needle", "here", { prepare }), "ok;1;0;0;1");
    assert.deepEqual(paragraph.scrolledWith, { block: "center", inline: "nearest" });
});

test("a page that replaces RegExp.prototype.exec gets a wrong answer, not an exception", () => {
    const document = page(h("p", "needle"));
    const prepare = "RegExp.prototype.exec = function () { throw new Error('mine'); };";

    assert.equal(find(document, "needle", "here", { prepare }), "fail");
});

test("opening reads a short selection and where it is", () => {
    const paragraph = h("p", "pick the needle here");
    const document = page(paragraph);
    const range = document.createRange();
    range.setStart(paragraph.firstChild, 9);
    range.setEnd(paragraph.firstChild, 15);
    document.getSelection().addRange(range);

    assert.equal(run(document, { mode: "begin", restore: null }), "sel;1.0.0:9|1.0.0:15;needle");
});

test("opening ignores long or multi-line selections and reports none", () => {
    const paragraph = h("p", "line one\nline two");
    const document = page(paragraph);
    assert.equal(run(document, { mode: "begin", restore: null }), "sel;;");

    const range = document.createRange();
    range.setStart(paragraph.firstChild, 0);
    range.setEnd(paragraph.firstChild, 17);
    document.getSelection().addRange(range);
    assert.equal(run(document, { mode: "begin", restore: null }), "sel;1.0.0:0|1.0.0:17;");
});

test("closing puts the page's selection back only while a match is selected", () => {
    const paragraph = h("p", "keep this; find the needle");
    const document = page(paragraph);
    const end = { mode: "end", query: "needle", matchCase: false, restore: "1.0.0:0|1.0.0:4", step: "here", current: -1 };

    find(document, "needle");
    assert.equal(run(document, end), "end;1");
    assert.equal(selected(document), "keep");

    find(document, "needle");
    const elsewhere = document.createRange();
    elsewhere.setStart(paragraph.firstChild, 5);
    elsewhere.setEnd(paragraph.firstChild, 9);
    document.getSelection().removeAllRanges();
    document.getSelection().addRange(elsewhere);
    assert.equal(run(document, end), "end;0");
    assert.equal(selected(document), "this");
});

test("closing rejects a saved position that no longer exists", () => {
    const document = page(h("p", "needle"));
    find(document, "needle");

    assert.equal(
        run(document, { mode: "end", query: "needle", matchCase: false, restore: "9.9.9:0|9.9.9:1", step: "here", current: -1 }),
        "end;0",
    );
    assert.equal(selected(document), "needle");
});

test("several presses at once move that many matches", () => {
    const document = page(h("p", "x1 x2 x3 x4"));

    assert.equal(find(document, "x"), "ok;4;0;0;1");
    assert.equal(find(document, "x", "next", { repeat: 2 }), "ok;4;2;0;1");
    assert.equal(find(document, "x", "previous", { repeat: 3 }), "ok;4;3;0;1");
});

test("previous with no selection goes to the match just above the view", () => {
    const top = h("p", "x2");
    const document = page(h("p", "x1"), top, h("p", "x3"));
    document._atTop = top;

    assert.equal(find(document, "x", "previous"), "ok;3;0;0;1");
    document.getSelection().removeAllRanges();
    assert.equal(find(document, "x"), "ok;3;1;0;1", "a new search starts at the top of the view");
});

test("a match on screen inside an element taller than the view is not scrolled to", () => {
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

    assert.equal(find(document, "needle", "here", { window }), "ok;2;0;0;1");
    assert.equal(window.scrolledBy, null, "the selected match is already on screen");
    assert.equal(pre.scrolledWith, null, "no jump to the middle of the element");

    assert.equal(find(document, "needle", "next", { window }), "ok;2;1;0;1");
    const offset = text.data.lastIndexOf("needle");
    const expected = -4000 + (offset / text.data.length) * 10000 - 300;
    assert.equal(window.scrolledBy[0], 0);
    assert.ok(Math.abs(window.scrolledBy[1] - expected) < 1, `scrolled by ${window.scrolledBy[1]}, expected ${expected}`);
});

test("the time limit also stops a run of matches that are not rendered", () => {
    // 30,000 matches in an element that is not rendered, each needing no new rendering check.
    const document = page(h("div", { rendered: false }, "x ".repeat(30000)), h("p", "x"));
    let ticks = 0;
    const clock = () => ticks++;

    assert.equal(find(document, "x", "here", { clock }), "ok;0;-1;1;0");
});

test("a long text node that is not rendered is skipped without being read", () => {
    const hidden = "needle ".repeat(20000);
    const document = page(h("div", { rendered: false }, hidden), h("p", "needle"));
    dataReads = [];

    assert.equal(find(document, "needle"), "ok;1;0;0;1");
    assert.ok(!dataReads.includes(hidden.length), "the hidden text was not read");
});

test("the limit cuts the side of the page farthest from the selection", () => {
    const filler = [];
    for (let i = 0; i < 40; i++) {
        filler.push(h("p", "filler ".repeat(8000)));
    }
    const last = h("p", "the last needle");
    const document = page(h("p", "the first needle"), ...filler, last);

    // Typing, with the selection at the end: the end is searched, the far beginning is cut.
    select(document, last.firstChild, 4);
    assert.equal(find(document, "needle"), "ok;1;-1;1;1");
    assert.equal(selected(document), "needle");
    assert.equal(document.getSelection().getRangeAt(0).startContainer, last.firstChild);

    // Return gets a larger allowance and reaches both.
    select(document, last.firstChild, 4);
    assert.equal(find(document, "needle", "here", { thorough: true }), "ok;2;1;0;1");
});

test("a huge text node is read in part around the selection", () => {
    const big = "a".repeat(3000000) + " needle " + "b".repeat(3000000);
    const pre = h("pre", { height: 100000 }, big);
    const document = page(pre);
    select(document, pre.firstChild, 3000001);
    dataReads = [];

    assert.equal(find(document, "needle"), "ok;1;-1;1;1");
    assert.equal(selected(document), "needle");
    assert.ok(!dataReads.includes(big.length), "the node was not read whole");
});

test("text directly inside display: contents counts as rendered", () => {
    const document = page(h("div", h("span", { rendered: false, display: "contents" }, "needle")));

    assert.equal(find(document, "needle"), "ok;1;0;0;1");
});

test("text in the shadow root of a hidden host is not counted", () => {
    const host = h("div", { rendered: false });
    host.attachShadow("needle in a hidden host");
    const shown = h("div");
    shown.attachShadow("needle in a shown host");
    const document = page(host, shown);

    assert.equal(find(document, "needle"), "ok;1;0;0;1");
});

test("an explicit search cut short with nothing found continues from where it stopped next time", () => {
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

    assert.equal(find(document, "needle", "here", press), "ok;0;-1;1;0", "the first round runs out of room");
    const caret = document.getSelection().getRangeAt(0);
    assert.notEqual(caret.startContainer, start.firstChild, "a caret marks where it stopped");

    assert.equal(find(document, "needle", "here", press), "ok;1;-1;1;1", "the next round goes on from there");
    assert.equal(selected(document), "needle");
});

test("setting up the walk does not run through every sibling of the start", () => {
    const container = h("div");
    for (let i = 0; i < 300000; i++) {
        container.append(h("p", "a"));
    }
    const last = h("p", "the needle");
    container.append(last);
    const document = page(container);
    select(document, last.firstChild, 0);
    calls.nextSibling = 0;

    assert.equal(find(document, "needle"), "ok;1;-1;1;1", "found; the far side is cut");
    assert.ok(calls.nextSibling < 1000, `nextSibling was read ${calls.nextSibling} times`);
});

test("a match that runs across the start point is found", () => {
    const bold = h("b", "dle");
    const document = page(h("p", "a nee", bold, " here"));
    select(document, bold.firstChild, 1);

    assert.equal(find(document, "needle"), "ok;1;0;0;1");
    assert.equal(selected(document), "needle");
});

test("only an explicit search leaves a caret where it stopped", () => {
    const filler = [];
    for (let i = 0; i < 40; i++) {
        filler.push(h("p", "filler ".repeat(8000)));
    }
    const start = h("p", "start here");
    const document = page(start, ...filler, h("p", "the needle at the end"));
    select(document, start.firstChild, 2, 5);

    assert.equal(find(document, "needle"), "ok;0;-1;1;0");
    assert.equal(selected(document), "art", "typing left the page's selection alone");

    assert.equal(find(document, "needle", "here", { thorough: true }).startsWith("ok;"), true);
});

test("closing after a caret puts the page's own selection back", () => {
    const start = h("p", "keep this");
    const filler = [];
    for (let i = 0; i < 40; i++) {
        filler.push(h("p", "filler ".repeat(60000)));
    }
    const document = page(start, ...filler, h("p", "needle"));
    select(document, start.firstChild, 0, 4);
    assert.equal(find(document, "needle", "here", { thorough: true }), "ok;0;-1;1;0");
    assert.equal(selected(document), "", "a caret replaced the selection");

    const end = { mode: "end", query: "needle", matchCase: false, restore: "1.0.0:0|1.0.0:4", caretLeft: true, step: "here", current: -1 };
    assert.equal(run(document, end), "end;1");
    assert.equal(selected(document), "keep");

    select(document, start.firstChild, 0, 0);
    assert.equal(
        run(document, { ...end, restore: null }),
        "end;1",
    );
    assert.equal(document.getSelection().rangeCount, 0, "with nothing to restore, the caret goes");
});

test("in a tall element the place is estimated from the offset in the whole element", () => {
    const pre = h("pre", { top: -4000, height: 10000 }, "a".repeat(5000), "b".repeat(4000) + " needle " + "c".repeat(992));
    const document = page(pre);
    const window = fakeWindow();

    assert.equal(find(document, "needle", "here", { window, thorough: true }), "ok;1;0;0;1");
    const offset = 5000 + 4001;
    const expected = -4000 + (offset / 10000) * 10000 - 300;
    assert.ok(window.scrolledBy !== null && Math.abs(window.scrolledBy[1] - expected) < 1,
        `scrolled by ${window.scrolledBy?.[1]}, expected ${expected}`);
});

test("a page that wraps RegExp and Reflect.apply sees the query, as the header says", () => {
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
    const context = vm.createContext({ document, window: fakeWindow(), performance: { now: () => 0 } });
    vm.runInContext(prepare, context);
    const answer = vm.runInContext(`(${script})(${JSON.stringify({
        mode: "find", query: "needle", matchCase: false, step: "here", repeat: 1, thorough: false, current: -1, restore: null,
    })})`, context);

    assert.equal(answer, "ok;1;0;0;1", "pass-through wrappers do not change the answer");
    const seen = vm.runInContext("seen", context);
    assert.ok(seen.includes("needle"), "the page's RegExp received the query");
    assert.ok(seen.includes("apply"), "the page's Reflect.apply was called");
});
