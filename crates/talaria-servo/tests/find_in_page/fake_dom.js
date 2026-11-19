// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Test fixture for `find_in_page_script.rs`, run in boa_engine (no Node.js): a small fake DOM, the
// helpers the cases use, and the cases' registry. Not product code: the product's only page script
// is `src/find_in_page.js`, which `__findSource` holds.

const cases = {};

function defineCase(name, body) {
    cases[name] = body;
}

function runCase(name) {
    const body = cases[name];
    if (body === undefined) {
        throw new Error(`no case named ${JSON.stringify(name)}`);
    }
    body();
    return "passed";
}

function caseNames() {
    return Object.keys(cases).join("\n");
}

function describe(value) {
    const json = JSON.stringify(value);
    return json === undefined ? String(value) : json;
}

function assertEqual(actual, expected, message = "values differ") {
    if (actual !== expected) {
        throw new Error(`${message}: expected ${describe(expected)}, got ${describe(actual)}`);
    }
}

function assertNotEqual(actual, unexpected, message = "values are equal") {
    if (actual === unexpected) {
        throw new Error(`${message}: got ${describe(actual)}`);
    }
}

function assertDeepEqual(actual, expected, message = "values differ") {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${message}: expected ${describe(expected)}, got ${describe(actual)}`);
    }
}

function assertTrue(value, message = "expected true") {
    if (!value) {
        throw new Error(message);
    }
}

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
    globalThis.document = document;
    globalThis.window = window;
    globalThis.performance = { now: clock };
    if (prepare !== undefined) {
        (0, eval)(prepare);
    }
    const before = Object.getOwnPropertyNames(globalThis).sort().join(",");
    // Called the way find_in_page.rs calls it: the script, then the request as a JSON literal.
    const answer = (0, eval)(`(${__findSource})(${JSON.stringify(request)})`);
    assertEqual(Object.getOwnPropertyNames(globalThis).sort().join(","), before, "no globals are left behind");
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
