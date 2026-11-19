// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Find in page. Servo 0.8.0 has no find API, so Mercury runs this through
// `WebView::evaluate_javascript`, which evaluates it in the top-level document only, in the page's own
// world (there is no isolated world). The file is one expression, a function that `find_in_page.rs`
// calls with a single request object; it answers with a short string.
//
// The page can see this script and its effects, and can interfere with it.
// - The page can always learn the query: the script selects the match in the page's own document,
//   and the page can read its selection. It can also observe the query directly: `Reflect`,
//   `Object`, `RegExp` and `performance` are read from the page's globals at the start of each
//   call (there is no realm-safe source for them short of creating a new realm, such as an
//   iframe, which the page would see too), and the query is given to `RegExp`. A page that
//   replaced `RegExp` beforehand receives the query.
// - The DOM built-ins (node, element, range and selection accessors and methods) are read once per
//   call from the prototype chains of real objects (`document`, its root element, a new text node,
//   a new range, the selection), never from globals such as `window.Range`, and are called only
//   through the saved `Reflect.apply`. Dictionaries passed to the engine have no prototype.
// - The script keeps no state between calls and leaves no globals behind (the page's selection
//   marks the current match), never evaluates text that came from the page, and answers with
//   primitives only.
// A page that replaced built-ins before the call can therefore make find fail, report wrong
// counts, or see the query, on that page. It cannot reach Mercury through this: the answer is parsed
// strictly.
//
// Work is bounded per call. A search started by typing gets TYPING_LIMITS, an explicit one (Return,
// a button, a shortcut) THOROUGH_LIMITS: a number of nodes, a time (checked in the walk and in the
// matching), and a number of characters of text (a longer text node is read in part, around the
// selection if it holds the selection). At most MAX_MATCHES matches are counted. The walk starts at
// the selection (or the top of the view) and goes forward to the end of the document, then backward
// from the start point to the beginning, so a limit cuts the parts farthest from the user. Hitting a
// limit marks the answer as truncated, and the position of the current match is then unknown.
(function (request) {
    "use strict";

    var TYPING_LIMITS = { nodes: 200000, milliseconds: 50, characters: 2000000 };
    // An explicit search runs in rounds of at most this long; the browser sends further rounds
    // itself when one finds nothing (see `CONTINUATION_ROUNDS` in talaria-core), giving the page's
    // thread a break between them.
    var THOROUGH_LIMITS = { nodes: 2000000, milliseconds: 200, characters: 16000000 };
    // Positions kept at each end of the matches found; all matches are counted, up to MAX_COUNT.
    var MAX_MATCHES = 1000;
    var MAX_COUNT = 1000000;
    var MAX_SELECTION_TEXT = 256;
    var MAX_PATH_DEPTH = 256;
    // Text nodes at least this long are checked for being rendered before they are read.
    var LARGE_TEXT = 65536;
    // About one line: a match this far above the bottom of the view counts as on screen.
    var LINE_HEIGHT = 24;
    // How far the walk's start moves back over inline content before the selection.
    var MAX_INLINE_STEPS = 64;
    // A tall element's text children are added up for the scroll estimate only up to this many.
    var MAX_TEXT_CHILDREN = 10000;
    // Between blocks: never part of a match, so matches stay inside one block of text.
    var BLOCK_SEPARATOR = "\u0000";

    var KIND_INLINE = 0;
    var KIND_BLOCK = 1;
    var KIND_SKIP = 2;
    var KIND_BREAK = 3;

    try {
        return run();
    } catch (error) {
        return "fail";
    }

    function run() {
        var apply = Reflect.apply;
        var getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
        var getPrototypeOf = Object.getPrototypeOf;
        var createObject = Object.create;
        var RegExpConstructor = RegExp;
        var regExpExec = RegExp.prototype.exec;
        var doc = document;

        function lookup(object, name, getter) {
            var target = object;
            for (var depth = 0; target !== null && depth < 16; depth++) {
                var descriptor = getOwnPropertyDescriptor(target, name);
                if (descriptor !== undefined) {
                    var found = getter ? descriptor.get : descriptor.value;
                    return typeof found === "function" ? found : null;
                }
                target = getPrototypeOf(target);
            }
            return null;
        }

        function required(object, name, getter) {
            var found = lookup(object, name, getter);
            if (found === null) {
                throw 0;
            }
            return found;
        }

        var nodeType = required(doc, "nodeType", true);
        var firstChild = required(doc, "firstChild", true);
        var nextSibling = required(doc, "nextSibling", true);
        var previousSibling = required(doc, "previousSibling", true);
        var lastChild = required(doc, "lastChild", true);
        var parentNode = required(doc, "parentNode", true);
        var documentElement = required(doc, "documentElement", true);
        var createRange = required(doc, "createRange", false);
        var createTextNode = required(doc, "createTextNode", false);
        var getSelection = required(doc, "getSelection", false);
        var performanceNow = typeof performance === "object" && performance !== null
            ? lookup(performance, "now", false)
            : null;

        var root = apply(documentElement, doc, []);
        if (root === null) {
            return request.mode === "begin" ? "sel;;" : request.mode === "end" ? "end;0" : "ok;0;-1;0;0";
        }
        var localName = required(root, "localName", true);
        var hasAttribute = required(root, "hasAttribute", false);
        var getClientRects = required(root, "getClientRects", false);
        var getBoundingClientRect = required(root, "getBoundingClientRect", false);
        var scrollIntoView = required(root, "scrollIntoView", false);
        var clientHeight = required(root, "clientHeight", true);
        var clientWidth = required(root, "clientWidth", true);
        var shadowRoot = lookup(root, "shadowRoot", true);
        var textData = required(apply(createTextNode, doc, [""]), "data", true);
        var probeRange = apply(createRange, doc, []);
        var setStart = required(probeRange, "setStart", false);
        var setEnd = required(probeRange, "setEnd", false);
        var startContainer = required(probeRange, "startContainer", true);
        var startOffset = required(probeRange, "startOffset", true);
        var endContainer = required(probeRange, "endContainer", true);
        var endOffset = required(probeRange, "endOffset", true);
        var selection = apply(getSelection, doc, []);
        if (selection === null || typeof selection !== "object") {
            throw 0;
        }
        var rangeCount = required(selection, "rangeCount", true);
        var getRangeAt = required(selection, "getRangeAt", false);
        var removeAllRanges = required(selection, "removeAllRanges", false);
        var addRange = required(selection, "addRange", false);
        var selectionToString = required(selection, "toString", false);
        var textLength = required(apply(createTextNode, doc, [""]), "length", true);
        var substringData = required(apply(createTextNode, doc, [""]), "substringData", false);
        var elementFromPoint = lookup(doc, "elementFromPoint", false);
        var getComputedStyle = lookup(window, "getComputedStyle", false);
        var scrollBy = lookup(window, "scrollBy", false);
        var limits = request.thorough === true ? THOROUGH_LIMITS : TYPING_LIMITS;

        var startedAt = now();

        // Dictionaries for the engine with no prototype, so converting them reads nothing the page
        // defined on `Object.prototype`.
        function dictionary(name, value, otherName, otherValue) {
            var result = createObject(null);
            result[name] = value;
            if (otherName !== undefined) {
                result[otherName] = otherValue;
            }
            return result;
        }

        function now() {
            return performanceNow === null ? 0 : apply(performanceNow, performance, []);
        }

        function currentRange() {
            if (apply(rangeCount, selection, []) < 1) {
                return null;
            }
            return apply(getRangeAt, selection, [0]);
        }

        function selectedText() {
            var text = apply(selectionToString, selection, []);
            return typeof text === "string" ? text : "";
        }

        if (request.mode === "begin") {
            return begin();
        }
        if (request.mode === "end") {
            return end();
        }
        return find();

        // The selection the page had when the bar opened: its text, if short and on one line, to
        // prefill the field, and where it was, so `end` can put it back.
        function begin() {
            var range = currentRange();
            if (range === null) {
                return "sel;;";
            }
            var saved = savePosition(apply(startContainer, range, []), apply(startOffset, range, []));
            var savedEnd = savePosition(apply(endContainer, range, []), apply(endOffset, range, []));
            var token = saved === null || savedEnd === null ? "" : saved + "|" + savedEnd;
            var text = trimmed(selectedText());
            if (text.length > MAX_SELECTION_TEXT || hasLineBreak(text)) {
                text = "";
            }
            return "sel;" + token + ";" + text;
        }

        // Puts back the page's own selection, but only while the page still has a match of the
        // query selected (otherwise the user or the page has moved on).
        function end() {
            if (typeof request.query !== "string" || request.query.length === 0) {
                return "end;0";
            }
            var whole = new RegExpConstructor("^" + pattern(request.query) + "$", flags());
            var matchSelected = apply(regExpExec, whole, [selectedText()]) !== null;
            // The caret an explicit search left where it stopped is a collapsed selection.
            var range = currentRange();
            var caret = request.caretLeft === true && range !== null &&
                apply(startContainer, range, []) === apply(endContainer, range, []) &&
                apply(startOffset, range, []) === apply(endOffset, range, []);
            if (!matchSelected && !caret) {
                return "end;0";
            }
            if (typeof request.restore !== "string") {
                if (caret) {
                    apply(removeAllRanges, selection, []);
                    return "end;1";
                }
                return "end;0";
            }
            var parts = splitOnce(request.restore, "|");
            if (parts === null) {
                return "end;0";
            }
            var start = resolvePosition(parts[0]);
            var finish = resolvePosition(parts[1]);
            if (start === null || finish === null) {
                return "end;0";
            }
            var restored = apply(createRange, doc, []);
            apply(setStart, restored, [start.node, start.offset]);
            apply(setEnd, restored, [finish.node, finish.offset]);
            apply(removeAllRanges, selection, []);
            apply(addRange, selection, [restored]);
            return "end;1";
        }

        function overTime() {
            return now() - startedAt > limits.milliseconds;
        }

        // Where the walk starts: the node holding the selection, else the element at the top of the
        // view, else the root. Moved up out of shadow trees, slotted content and skipped elements,
        // so that the path to the root is plain light DOM, then back over the inline content just
        // before it (at most MAX_INLINE_STEPS siblings and ancestors, never past a block), so that a
        // match running across the start point is still one piece of text. Not to the start of the
        // block itself: a caret left inside a long block must let the next round go further.
        function startNode(anchorNode) {
            var start = anchorNode;
            if (start === null && elementFromPoint !== null) {
                start = apply(elementFromPoint, doc, [apply(clientWidth, root, []) / 2, 1]);
            }
            if (start === null || start === undefined) {
                return root;
            }
            for (var attempt = 0; attempt < MAX_PATH_DEPTH; attempt++) {
                var moved = null;
                var depth = 0;
                for (var node = start; node !== root; node = apply(parentNode, node, [])) {
                    if (node === null || depth++ > MAX_PATH_DEPTH) {
                        return root;
                    }
                    var type = apply(nodeType, node, []);
                    if (type === 11) {
                        moved = hostOf(node);
                        if (moved === null) {
                            return root;
                        }
                    } else if (node !== start && type === 1 && (
                        (shadowRoot !== null && apply(shadowRoot, node, []) !== null) ||
                        classify(apply(localName, node, [])) === KIND_SKIP ||
                        apply(hasAttribute, node, ["hidden"]))) {
                        // Light content of a shadow host, or inside a skipped element: start at the
                        // outermost such element, which the walk then handles as usual.
                        moved = node;
                    }
                }
                if (moved === null) {
                    break;
                }
                start = moved;
            }
            var node = start;
            for (var steps = 0; steps < MAX_INLINE_STEPS && node !== root; steps++) {
                if (isBlock(node)) {
                    break;
                }
                var previous = apply(previousSibling, node, []);
                if (previous !== null) {
                    if (isBlock(previous)) {
                        break;
                    }
                    node = previous;
                    continue;
                }
                var parent = apply(parentNode, node, []);
                if (parent === null || parent === root || apply(nodeType, parent, []) !== 1 || isBlock(parent)) {
                    break;
                }
                node = parent;
            }
            return node;
        }

        function isBlock(node) {
            return apply(nodeType, node, []) === 1 && classify(apply(localName, node, [])) === KIND_BLOCK;
        }

        // The ancestors of `start`, from the root down to `start`.
        function pathTo(start) {
            var reversed = [];
            var count = 0;
            for (var node = start; node !== root; node = apply(parentNode, node, [])) {
                reversed[count++] = node;
            }
            reversed[count++] = root;
            var path = [];
            for (var i = 0; i < count; i++) {
                path[i] = reversed[count - 1 - i];
            }
            return path;
        }

        function find() {
            if (typeof request.query !== "string" || request.query.length === 0) {
                return "ok;0;-1;0;0";
            }
            var anchorRange = currentRange();
            var anchorNode = anchorRange === null ? null : apply(startContainer, anchorRange, []);
            var anchorOffset = anchorRange === null ? 0 : apply(startOffset, anchorRange, []);
            var start = startNode(anchorNode);
            var path = pathTo(start);
            var truncated = false;
            var visited = 0;
            var characters = 0;
            // What each stack entry is: a node to handle, a block's end, or a cursor (a node and the
            // siblings after it, forward, or before it, backward; or a position in a slot's list),
            // so that siblings are reached one at a time and never listed up front. Kept apart from
            // the entries, so nothing the page defines on its nodes is read to tell them apart.
            var ITEM_NODE = 0;
            var ITEM_BLOCK_END = 1;
            var ITEM_CURSOR = 2;
            var ITEM_LIST = 3;
            // The last node the forward walk handled, and the offset just after what it read.
            var lastNode = null;
            var lastOffset = 0;

            // One walk: forward from `start` to the end of the document, or backward from just
            // before `start` to its beginning. Records text nodes (with the part read) and block
            // separators, in walk order. Every node it touches counts against the limits.
            function walk(forward) {
                var entries = [];
                var entryCount = 0;
                var stack = [];
                var kinds = [];
                var indices = [];
                var top = 0;
                var step = forward ? nextSibling : previousSibling;
                function push(kind, value, index) {
                    stack[top] = value;
                    kinds[top] = kind;
                    indices[top] = index;
                    top++;
                }
                for (var level = 0; level + 1 < path.length; level++) {
                    if (classify(apply(localName, path[level], [])) === KIND_BLOCK) {
                        push(ITEM_BLOCK_END, null, 0);
                    }
                    var sibling = apply(step, path[level + 1], []);
                    if (sibling !== null) {
                        push(ITEM_CURSOR, sibling, 0);
                    }
                }
                if (forward) {
                    push(ITEM_NODE, start, 0);
                }
                while (top > 0) {
                    top--;
                    var item = stack[top];
                    var itemKind = kinds[top];
                    var itemIndex = indices[top];
                    stack[top] = undefined;
                    if (itemKind === ITEM_BLOCK_END) {
                        entries[entryCount++] = null;
                        continue;
                    }
                    if (itemKind === ITEM_CURSOR) {
                        var next = apply(step, item, []);
                        if (next !== null) {
                            push(ITEM_CURSOR, next, 0);
                        }
                    } else if (itemKind === ITEM_LIST) {
                        var list = item;
                        item = list[itemIndex];
                        var following = itemIndex + (forward ? 1 : -1);
                        if (following >= 0 && following < list.length) {
                            push(ITEM_LIST, list, following);
                        }
                    }
                    visited++;
                    if (visited > limits.nodes || ((visited & 1023) === 0 && overTime())) {
                        truncated = true;
                        break;
                    }
                    var type = apply(nodeType, item, []);
                    if (type === 3) {
                        var length = apply(textLength, item, []);
                        if (length === 0) {
                            continue;
                        }
                        if (length >= LARGE_TEXT && !isRendered(item)) {
                            continue;
                        }
                        var room = limits.characters - characters;
                        var base = 0;
                        var data;
                        if (length <= room) {
                            data = apply(textData, item, []);
                        } else {
                            // Read only part of a text node too long for what is left: around the
                            // selection, else the part nearest the start point.
                            truncated = true;
                            if (item === anchorNode) {
                                base = anchorOffset - (room >> 1);
                                base = base < 0 ? 0 : base > length - room ? length - room : base;
                            } else if (!forward) {
                                base = length - room;
                            }
                            data = room > 0 ? apply(substringData, item, [base, room]) : "";
                        }
                        characters += data.length;
                        entries[entryCount++] = { node: item, base: base, data: data, length: length };
                        if (forward) {
                            lastNode = item;
                            lastOffset = base + data.length;
                        }
                        if (truncated) {
                            break;
                        }
                        continue;
                    }
                    if (type !== 1) {
                        continue;
                    }
                    var name = apply(localName, item, []);
                    var kind = classify(name);
                    if (forward) {
                        lastNode = item;
                        lastOffset = 0;
                    }
                    if (kind === KIND_SKIP || apply(hasAttribute, item, ["hidden"])) {
                        continue;
                    }
                    if (kind === KIND_BREAK) {
                        entries[entryCount++] = "\n";
                        continue;
                    }
                    // An open shadow root is walked instead of the host's light children; a slot
                    // stands for its assigned nodes (or its own children when nothing is assigned).
                    // Both are bounded like blocks, so a match never spans two trees (a Range
                    // cannot).
                    var source = item;
                    var assigned = null;
                    var shadow = shadowRoot === null ? null : apply(shadowRoot, item, []);
                    if (shadow !== null && shadow !== undefined) {
                        source = shadow;
                        kind = KIND_BLOCK;
                    } else if (name === "slot") {
                        var assignedNodes = lookup(item, "assignedNodes", false);
                        assigned = assignedNodes === null
                            ? null
                            : apply(assignedNodes, item, [dictionary("flatten", true)]);
                        if (assigned !== null && assigned.length === 0) {
                            assigned = null;
                        }
                        kind = KIND_BLOCK;
                    }
                    if (kind === KIND_BLOCK) {
                        entries[entryCount++] = null;
                        push(ITEM_BLOCK_END, null, 0);
                    }
                    if (assigned !== null) {
                        push(ITEM_LIST, assigned, forward ? 0 : assigned.length - 1);
                    } else {
                        var child = apply(forward ? firstChild : lastChild, source, []);
                        if (child !== null) {
                            push(ITEM_CURSOR, child, 0);
                        }
                    }
                }
                return { list: entries, count: entryCount };
            }

            // The text as one string in document order from `start` (the forward walk), then a
            // separator, then the beginning of the document up to `start` (the backward walk,
            // reversed). `starts[i]` is where text node `i` begins in it.
            var text = "";
            var nodes = [];
            var starts = [];
            var bases = [];
            var lengths = [];
            var spans = [];
            var nodeCount = 0;
            var anchor = -1;

            function append(entry) {
                if (entry === null) {
                    text += BLOCK_SEPARATOR;
                } else if (typeof entry === "string") {
                    text += entry;
                } else {
                    if (entry.node === anchorNode && anchorOffset >= entry.base) {
                        anchor = text.length + anchorOffset - entry.base;
                    }
                    nodes[nodeCount] = entry.node;
                    starts[nodeCount] = text.length;
                    bases[nodeCount] = entry.base;
                    lengths[nodeCount] = entry.length;
                    spans[nodeCount] = entry.data.length;
                    nodeCount++;
                    text += entry.data;
                }
            }

            var forwardEntries = walk(true);
            var forwardCut = truncated;
            for (var f = 0; f < forwardEntries.count; f++) {
                append(forwardEntries.list[f]);
            }
            text += BLOCK_SEPARATOR;
            var backwardStart = text.length;
            if (!truncated) {
                var backwardEntries = walk(false);
                for (var b = backwardEntries.count - 1; b >= 0; b--) {
                    append(backwardEntries.list[b]);
                }
            }

            // The matches, skipping those in elements that are not rendered. All are counted (up to
            // MAX_COUNT, within the time limit); the positions of the first MAX_MATCHES and of the
            // last MAX_MATCHES are kept, which is all that choosing the current match needs.
            var expression = new RegExpConstructor(pattern(request.query), "g" + flags());
            var headStarts = [];
            var headEnds = [];
            var tailStarts = [];
            var tailEnds = [];
            var tailNext = 0;
            var count = 0;
            var forwardMatches = 0;
            var visibility = [];
            for (var iteration = 1; ; iteration++) {
                if ((iteration & 63) === 0 && overTime()) {
                    truncated = true;
                    break;
                }
                var found = apply(regExpExec, expression, [text]);
                if (found === null) {
                    break;
                }
                var matchStart = found.index;
                var matchLength = found[0].length;
                if (matchLength === 0) {
                    expression.lastIndex = matchStart + 1;
                    continue;
                }
                var startIndex = nodeAt(starts, nodeCount, matchStart);
                if (startIndex < 0) {
                    continue;
                }
                if (visibility[startIndex] === undefined) {
                    visibility[startIndex] = isRendered(nodes[startIndex]);
                }
                if (!visibility[startIndex]) {
                    continue;
                }
                if (count === MAX_COUNT) {
                    truncated = true;
                    break;
                }
                if (count < MAX_MATCHES) {
                    headStarts[count] = matchStart;
                    headEnds[count] = matchStart + matchLength;
                } else {
                    tailStarts[tailNext] = matchStart;
                    tailEnds[tailNext] = matchStart + matchLength;
                    tailNext = tailNext + 1 === MAX_MATCHES ? 0 : tailNext + 1;
                }
                if (matchStart < backwardStart) {
                    forwardMatches++;
                }
                count++;
            }

            if (count === 0) {
                if (truncated && request.thorough === true) {
                    // An explicit search that found nothing before a limit leaves a caret where the
                    // walk stopped (the end of what the forward walk read, else the top of the
                    // document, which the backward walk did not reach), so the next search goes on
                    // from there. Searches made while typing leave the page's selection alone.
                    try {
                        var caret = apply(createRange, doc, []);
                        var caretNode = forwardCut && lastNode !== null ? lastNode : root;
                        var caretOffset = forwardCut && lastNode !== null ? lastOffset : 0;
                        apply(setStart, caret, [caretNode, caretOffset]);
                        apply(setEnd, caret, [caretNode, caretOffset]);
                        apply(removeAllRanges, selection, []);
                        apply(addRange, selection, [caret]);
                    } catch (error) {
                        // Only the continuation is lost.
                    }
                }
                return "ok;0;-1;" + (truncated ? 1 : 0) + ";0";
            }

            // The kept start and end of match `index` (in the order found), or -1 if not kept.
            var tailKept = count > MAX_MATCHES ? smaller(count - MAX_MATCHES, MAX_MATCHES) : 0;
            var firstTail = count - tailKept;
            function kept(index) {
                return index < MAX_MATCHES || index >= firstTail;
            }
            function ringIndex(index) {
                return count - MAX_MATCHES <= MAX_MATCHES
                    ? index - MAX_MATCHES
                    : (tailNext + index - firstTail) % MAX_MATCHES;
            }
            function startOf(index) {
                return index < MAX_MATCHES ? headStarts[index] : tailStarts[ringIndex(index)];
            }
            function endOf(index) {
                return index < MAX_MATCHES ? headEnds[index] : tailEnds[ringIndex(index)];
            }

            // Matches from the forward walk come first; those from the backward walk come before
            // them in the document.
            var backwardMatches = count - forwardMatches;
            function documentIndex(index) {
                return index >= forwardMatches ? index - forwardMatches : index + backwardMatches;
            }
            function fromDocumentIndex(position) {
                return position < backwardMatches ? position + forwardMatches : position - backwardMatches;
            }

            var repeat = typeof request.repeat === "number" && request.repeat >= 1 ? request.repeat : 1;
            var current = chooseMatch(request.step, typeof request.current === "number" ? request.current : -1);
            if (!kept(current)) {
                current = MAX_MATCHES - 1;
            }
            var selected = 0;
            try {
                var first = nodeAt(starts, nodeCount, startOf(current));
                var last = nodeAt(starts, nodeCount, endOf(current) - 1);
                var range = apply(createRange, doc, []);
                // A match can begin or end on the line break of a `<br>`, which is in no text node.
                var startOffsetInNode = bases[first] + smaller(startOf(current) - starts[first], spans[first]);
                apply(setStart, range, [nodes[first], startOffsetInNode]);
                apply(setEnd, range, [nodes[last], bases[last] + smaller(endOf(current) - starts[last], spans[last])]);
                apply(removeAllRanges, selection, []);
                apply(addRange, selection, [range]);
                selected = 1;
                reveal(nodes[first], startOffsetInNode);
            } catch (error) {
                // The count stands; only selecting or scrolling failed.
            }
            var position = truncated ? -1 : documentIndex(current);
            return "ok;" + count + ";" + position + ";" + (truncated ? 1 : 0) + ";" + selected;

            // In cyclic order from the anchor: the matches after it, then the ones before it.
            function chooseMatch(step, previous) {
                var rank;
                if (anchor >= 0) {
                    var before = 0;
                    while (before < count && before < MAX_MATCHES && headStarts[before] < anchor) {
                        before++;
                    }
                    if (step === "previous") {
                        rank = count - repeat;
                    } else {
                        var at = before < count && before < MAX_MATCHES && headStarts[before] === anchor;
                        rank = step === "next" ? (at ? 1 : 0) + repeat - 1 : 0;
                    }
                    return (((rank % count) + count) % count + before) % count;
                }
                if (previous >= 0 && !truncated) {
                    var target = step === "next" ? previous + repeat
                        : step === "previous" ? previous - repeat
                        : previous < count ? previous : count - 1;
                    return fromDocumentIndex(((target % count) + count) % count);
                }
                if (step === "previous") {
                    return ((count - repeat) % count + count) % count;
                }
                rank = firstInView() + (step === "next" ? repeat - 1 : 0);
                return rank % count;
            }

            // The first match of the forward walk that is not above the view (matches there are in
            // document order, which is close enough to top-to-bottom for a binary search).
            function firstInView() {
                var low = 0;
                var limit = smaller(forwardMatches, MAX_MATCHES);
                var high = limit;
                while (low < high) {
                    var middle = (low + high) >> 1;
                    var index = nodeAt(starts, nodeCount, headStarts[middle]);
                    var estimate = matchPosition(nodes[index], bases[index] + headStarts[middle] - starts[index]);
                    if (estimate === null || estimate.bottom > 0) {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                return low < limit ? low : 0;
            }
        }

        // Whether the text node is rendered: its parent element has boxes; an element with
        // `display: contents` has none of its own, so its parent decides; text directly in a shadow
        // root goes by the host.
        function isRendered(textNode) {
            var parent = apply(parentNode, textNode, []);
            for (var depth = 0; depth < 64; depth++) {
                if (parent === null) {
                    return true;
                }
                var type = apply(nodeType, parent, []);
                if (type === 11) {
                    parent = hostOf(parent);
                    continue;
                }
                if (type !== 1) {
                    return true;
                }
                if (apply(getClientRects, parent, []).length > 0) {
                    return true;
                }
                if (displayOf(parent) !== "contents") {
                    return false;
                }
                parent = apply(parentNode, parent, []);
            }
            return true;
        }

        function hostOf(shadow) {
            var host = lookup(shadow, "host", true);
            return host === null ? null : apply(host, shadow, []);
        }

        function displayOf(element) {
            if (getComputedStyle === null) {
                return "";
            }
            var style = apply(getComputedStyle, window, [element]);
            var getPropertyValue = style === null ? null : lookup(style, "getPropertyValue", false);
            return getPropertyValue === null ? "" : apply(getPropertyValue, style, ["display"]);
        }

        // The match's estimated place in the view. In an element taller than the view (a long
        // `<pre>`, a `text/plain` page) the match is placed in proportion to its offset in the
        // element's text (its text children, summed), as Servo 0.8.0 cannot measure a range.
        function matchPosition(textNode, offset) {
            var element = apply(parentNode, textNode, []);
            if (element === null || apply(nodeType, element, []) !== 1) {
                return null;
            }
            var rect = apply(getBoundingClientRect, element, []);
            var height = apply(clientHeight, root, []);
            if (rect.height <= height) {
                return { element: element, rect: rect, tall: false, top: rect.top, bottom: rect.bottom };
            }
            var before = 0;
            var total = 0;
            var seen = false;
            var children = 0;
            for (var child = apply(firstChild, element, []); child !== null; child = apply(nextSibling, child, [])) {
                if (++children > MAX_TEXT_CHILDREN) {
                    // Too many to add up: the text node alone.
                    total = apply(textLength, textNode, []);
                    before = offset;
                    break;
                }
                if (apply(nodeType, child, []) !== 3) {
                    continue;
                }
                var childLength = apply(textLength, child, []);
                if (child === textNode) {
                    before = total + offset;
                    seen = true;
                }
                total += childLength;
            }
            if (total === 0 || (!seen && children <= MAX_TEXT_CHILDREN)) {
                return { element: element, rect: rect, tall: false, top: rect.top, bottom: rect.bottom };
            }
            var top = rect.top + (before / total) * rect.height;
            return { element: element, rect: rect, tall: true, top: top, bottom: top + LINE_HEIGHT };
        }

        // Brings the match into view, unless it is already on screen.
        function reveal(textNode, offset) {
            var place = matchPosition(textNode, offset);
            if (place === null) {
                return;
            }
            var height = apply(clientHeight, root, []);
            var width = apply(clientWidth, root, []);
            var rect = place.rect;
            if (!place.tall) {
                if (rect.top >= 0 && rect.bottom <= height && rect.left >= 0 && rect.right <= width) {
                    return;
                }
                apply(scrollIntoView, place.element, [dictionary("block", "center", "inline", "nearest")]);
                return;
            }
            // A tall element: scroll only if the estimated place is off screen, and then to it.
            if (rect.top < height && rect.bottom > 0 && place.top >= 0 && place.bottom <= height) {
                return;
            }
            if (scrollBy !== null) {
                apply(scrollBy, window, [0, place.top - height / 2]);
            }
        }

        // "1.0.3:5": child indices from the root element, then an offset.
        function savePosition(node, offset) {
            var steps = "";
            var depth = 0;
            var current = node;
            while (current !== root) {
                if (current === null || depth >= MAX_PATH_DEPTH) {
                    return null;
                }
                var parent = apply(parentNode, current, []);
                if (parent === null) {
                    return null;
                }
                var index = 0;
                for (var sibling = apply(firstChild, parent, []); sibling !== current; sibling = apply(nextSibling, sibling, [])) {
                    if (sibling === null) {
                        return null;
                    }
                    index++;
                }
                steps = depth === 0 ? "" + index : index + "." + steps;
                current = parent;
                depth++;
            }
            return steps + ":" + offset;
        }

        function resolvePosition(saved) {
            var parts = splitOnce(saved, ":");
            if (parts === null) {
                return null;
            }
            var node = root;
            var steps = parts[0];
            var number = -1;
            for (var i = 0; i <= steps.length; i++) {
                var code = i < steps.length ? steps.charCodeAt(i) : 46;
                if (code >= 48 && code <= 57) {
                    number = (number < 0 ? 0 : number * 10) + (code - 48);
                    continue;
                }
                if (code !== 46) {
                    return null;
                }
                if (number < 0) {
                    if (steps.length === 0) {
                        break;
                    }
                    return null;
                }
                var child = apply(firstChild, node, []);
                for (var k = 0; k < number && child !== null; k++) {
                    child = apply(nextSibling, child, []);
                }
                if (child === null) {
                    return null;
                }
                node = child;
                number = -1;
            }
            var offset = toNumber(parts[1]);
            return offset < 0 ? null : { node: node, offset: offset };
        }
    }

    function classify(name) {
        switch (name) {
            case "script": case "style": case "template": case "noscript": case "head":
            case "title": case "iframe": case "frame": case "frameset": case "object":
            case "embed": case "textarea": case "select": case "input": case "canvas":
            case "video": case "audio": case "img": case "picture": case "map": case "datalist":
                return KIND_SKIP;
            case "br":
                return KIND_BREAK;
            case "address": case "article": case "aside": case "blockquote": case "body":
            case "button": case "caption": case "center": case "dd": case "details": case "dialog":
            case "div": case "dl": case "dt": case "fieldset": case "figcaption": case "figure":
            case "footer": case "form": case "h1": case "h2": case "h3": case "h4": case "h5":
            case "h6": case "header": case "hgroup": case "hr": case "html": case "legend":
            case "li": case "main": case "menu": case "nav": case "ol": case "option": case "p":
            case "pre": case "section": case "summary": case "table": case "tbody": case "td":
            case "tfoot": case "th": case "thead": case "tr": case "ul":
                return KIND_BLOCK;
            default:
                return KIND_INLINE;
        }
    }

    // The query as a regular expression: special characters escaped, each run of white space
    // matching any run of white space.
    function pattern(query) {
        var source = "";
        var inSpace = false;
        for (var i = 0; i < query.length; i++) {
            var character = query.charAt(i);
            if (isSpace(query.charCodeAt(i))) {
                if (!inSpace) {
                    source += "\\s+";
                }
                inSpace = true;
                continue;
            }
            inSpace = false;
            switch (character) {
                case "\\": case "^": case "$": case ".": case "*": case "+": case "?": case "(":
                case ")": case "[": case "]": case "{": case "}": case "|": case "/":
                    source += "\\" + character;
                    break;
                case BLOCK_SEPARATOR:
                    break;
                default:
                    source += character;
            }
        }
        return source;
    }

    // Unicode (`u`) matching; without "match case", simple case folding (`i`), which keeps the
    // length of the text, so offsets map straight back to the page. No diacritic folding.
    function flags() {
        return request.matchCase === true ? "u" : "iu";
    }

    function smaller(a, b) {
        return a < b ? a : b;
    }

    function isSpace(code) {
        return code === 32 || code === 9 || code === 10 || code === 12 || code === 13 ||
            code === 160 || code === 0x2028 || code === 0x2029 || code === 0x3000 ||
            (code >= 0x2000 && code <= 0x200a) || code === 0x202f || code === 0x205f;
    }

    function hasLineBreak(text) {
        for (var i = 0; i < text.length; i++) {
            var code = text.charCodeAt(i);
            if (code === 10 || code === 13 || code === 0) {
                return true;
            }
        }
        return false;
    }

    function trimmed(text) {
        var start = 0;
        var end = text.length;
        while (start < end && isSpace(text.charCodeAt(start))) {
            start++;
        }
        while (end > start && isSpace(text.charCodeAt(end - 1))) {
            end--;
        }
        var result = "";
        for (var i = start; i < end; i++) {
            result += text.charAt(i);
        }
        return result;
    }

    function splitOnce(text, separator) {
        for (var i = 0; i < text.length; i++) {
            if (text.charAt(i) === separator) {
                var head = "";
                var tail = "";
                for (var j = 0; j < i; j++) {
                    head += text.charAt(j);
                }
                for (var k = i + 1; k < text.length; k++) {
                    tail += text.charAt(k);
                }
                return [head, tail];
            }
        }
        return null;
    }

    function toNumber(text) {
        if (text.length === 0 || text.length > 9) {
            return -1;
        }
        var value = 0;
        for (var i = 0; i < text.length; i++) {
            var code = text.charCodeAt(i);
            if (code < 48 || code > 57) {
                return -1;
            }
            value = value * 10 + (code - 48);
        }
        return value;
    }

    // The index of the text node containing `offset`, or -1 (a separator).
    function nodeAt(starts, nodeCount, offset) {
        var low = 0;
        var high = nodeCount - 1;
        var result = -1;
        while (low <= high) {
            var middle = (low + high) >> 1;
            if (starts[middle] <= offset) {
                result = middle;
                low = middle + 1;
            } else {
                high = middle - 1;
            }
        }
        return result;
    }
})
