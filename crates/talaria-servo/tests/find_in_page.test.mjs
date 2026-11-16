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
    get nextSibling() {
        if (this._parent === null) {
            return null;
        }
        const siblings = this._parent._children;
        return siblings[siblings.indexOf(this) + 1] ?? null;
    }
    get parentNode() {
        return this._parent;
    }
    append(...children) {
        for (const child of children) {
            const node = typeof child === "string" ? new FakeText(child) : child;
            node._parent = this;
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
        return this._data;
    }
}

class FakeText extends FakeCharacterData {}

class FakeElement extends FakeNode {
    constructor(name, options = {}) {
        super(1);
        this._name = name;
        this._attributes = options.attributes ?? {};
        this._rendered = options.rendered ?? true;
        this._top = options.top ?? 0;
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
        return { top: this._top, bottom: this._top + 20, left: 0, right: 100 };
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
        this._shadow = new FakeNode(11).append(...children);
        return this;
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

function run(document, request, prepare) {
    const context = vm.createContext({ document, performance: { now: () => 0 } });
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
        current: options.current ?? -1,
        restore: null,
    }, options.prepare);
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

test("the count stops at its limit and says so", () => {
    const document = page(h("p", "x ".repeat(1500)));

    assert.equal(find(document, "x"), "ok;1000;0;1;1");
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
