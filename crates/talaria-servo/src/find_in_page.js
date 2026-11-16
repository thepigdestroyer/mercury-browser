// Find in page. Servo 0.8.0 has no find API, so Mercury runs this through
// `WebView::evaluate_javascript`, which evaluates it in the top-level document only, in the page's own
// world (there is no isolated world). The file is one expression, a function that `find_in_page.rs`
// calls with a single request object; it answers with a short string.
//
// The page can see this script and its effects, and can interfere with it. The script therefore:
// - reads every built-in it uses once, at the start of each call, from the prototypes of real
//   objects (not from globals such as `window.Range`), and calls only those;
// - keeps no state between calls and leaves no globals behind (the page's selection marks the
//   current match);
// - never evaluates text that came from the page, and answers with primitives only.
// A page that replaced those built-ins before the call can still make find fail or report wrong
// counts on that page. It cannot reach Mercury through this: the answer is parsed strictly.
//
// Limits: at most MAX_NODES nodes are visited and the walk stops after TIME_BUDGET_MS; at most
// MAX_MATCHES matches are counted. Either limit marks the answer as truncated.
(function (request) {
    "use strict";

    var MAX_NODES = 200000;
    var MAX_MATCHES = 1000;
    var TIME_BUDGET_MS = 100;
    var MAX_SELECTION_TEXT = 256;
    var MAX_PATH_DEPTH = 256;
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
            if (apply(regExpExec, whole, [selectedText()]) === null) {
                return "end;0";
            }
            if (typeof request.restore !== "string") {
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
            var range = apply(createRange, doc, []);
            apply(setStart, range, [start.node, start.offset]);
            apply(setEnd, range, [finish.node, finish.offset]);
            apply(removeAllRanges, selection, []);
            apply(addRange, selection, [range]);
            return "end;1";
        }

        function find() {
            if (typeof request.query !== "string" || request.query.length === 0) {
                return "ok;0;-1;0;0";
            }
            var anchorRange = currentRange();
            var anchorNode = anchorRange === null ? null : apply(startContainer, anchorRange, []);
            var anchorOffset = anchorRange === null ? 0 : apply(startOffset, anchorRange, []);
            var anchor = -1;

            // The text of the document as one string, with each text node's start in it.
            var text = "";
            var nodes = [];
            var starts = [];
            var lengths = [];
            var nodeCount = 0;
            var truncated = false;
            var visited = 0;
            var blockEnd = {};
            var stack = [root];
            var top = 1;
            var children = [];

            while (top > 0) {
                var item = stack[--top];
                stack[top] = undefined;
                if (item === blockEnd) {
                    text += BLOCK_SEPARATOR;
                    continue;
                }
                visited++;
                if (visited > MAX_NODES || ((visited & 1023) === 0 && now() - startedAt > TIME_BUDGET_MS)) {
                    truncated = true;
                    break;
                }
                var type = apply(nodeType, item, []);
                if (type === 3) {
                    var data = apply(textData, item, []);
                    if (item === anchorNode) {
                        anchor = text.length + anchorOffset;
                    }
                    if (data.length > 0) {
                        nodes[nodeCount] = item;
                        starts[nodeCount] = text.length;
                        lengths[nodeCount] = data.length;
                        nodeCount++;
                        text += data;
                    }
                    continue;
                }
                if (type !== 1 && type !== 11) {
                    continue;
                }
                var kind = KIND_INLINE;
                var source = item;
                var assigned = null;
                if (type === 1) {
                    var name = apply(localName, item, []);
                    kind = classify(name);
                    if (kind === KIND_SKIP || apply(hasAttribute, item, ["hidden"])) {
                        continue;
                    }
                    if (kind === KIND_BREAK) {
                        text += "\n";
                        continue;
                    }
                    // Open shadow roots are searched in place of the light children they render;
                    // slots stand for the nodes assigned to them. Both are bounded like blocks, so a
                    // match never spans two trees (a Range cannot).
                    var shadow = shadowRoot === null ? null : apply(shadowRoot, item, []);
                    if (shadow !== null && shadow !== undefined) {
                        source = shadow;
                        kind = KIND_BLOCK;
                    } else if (name === "slot") {
                        var assignedNodes = lookup(item, "assignedNodes", false);
                        if (assignedNodes !== null) {
                            assigned = apply(assignedNodes, item, [dictionary("flatten", true)]);
                        }
                        kind = KIND_BLOCK;
                    }
                }
                if (kind === KIND_BLOCK) {
                    text += BLOCK_SEPARATOR;
                    stack[top++] = blockEnd;
                }
                var childCount = 0;
                if (assigned !== null && assigned.length > 0) {
                    for (var a = 0; a < assigned.length && a < MAX_NODES; a++) {
                        children[childCount++] = assigned[a];
                    }
                } else {
                    for (var child = apply(firstChild, source, []); child !== null; child = apply(nextSibling, child, [])) {
                        children[childCount++] = child;
                        if (childCount > MAX_NODES) {
                            break;
                        }
                    }
                }
                while (childCount > 0) {
                    stack[top++] = children[--childCount];
                    children[childCount] = undefined;
                }
            }

            // The matches, skipping those in elements that are not rendered.
            var expression = new RegExpConstructor(pattern(request.query), "g" + flags());
            var matchStarts = [];
            var matchEnds = [];
            var count = 0;
            var visibility = [];
            for (;;) {
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
                var startNode = nodeAt(starts, nodeCount, matchStart);
                if (startNode < 0 || !isRendered(startNode)) {
                    continue;
                }
                if (count === MAX_MATCHES || ((count & 63) === 63 && now() - startedAt > TIME_BUDGET_MS)) {
                    truncated = true;
                    break;
                }
                matchStarts[count] = matchStart;
                matchEnds[count] = matchStart + matchLength;
                count++;
            }

            function isRendered(index) {
                if (visibility[index] === undefined) {
                    var parent = apply(parentNode, nodes[index], []);
                    visibility[index] =
                        parent === null || apply(nodeType, parent, []) !== 1 ||
                        apply(getClientRects, parent, []).length > 0;
                }
                return visibility[index];
            }

            if (count === 0) {
                return "ok;0;-1;" + (truncated ? 1 : 0) + ";0";
            }

            var current = chooseMatch(request.step, anchor, typeof request.current === "number" ? request.current : -1);
            var selected = 0;
            try {
                var first = nodeAt(starts, nodeCount, matchStarts[current]);
                var last = nodeAt(starts, nodeCount, matchEnds[current] - 1);
                var range = apply(createRange, doc, []);
                // A match can begin or end on the line break of a `<br>`, which is in no text node.
                apply(setStart, range, [nodes[first], smaller(matchStarts[current] - starts[first], lengths[first])]);
                apply(setEnd, range, [nodes[last], smaller(matchEnds[current] - starts[last], lengths[last])]);
                apply(removeAllRanges, selection, []);
                apply(addRange, selection, [range]);
                selected = 1;
                reveal(nodes[first]);
            } catch (error) {
                // The count stands; only selecting or scrolling failed.
            }
            return "ok;" + count + ";" + current + ";" + (truncated ? 1 : 0) + ";" + selected;

            function chooseMatch(step, anchorAt, previous) {
                var index;
                if (anchorAt >= 0) {
                    if (step === "previous") {
                        for (index = count - 1; index >= 0 && matchStarts[index] >= anchorAt; index--) {}
                        return index >= 0 ? index : count - 1;
                    }
                    for (index = 0; index < count; index++) {
                        if (step === "next" ? matchStarts[index] > anchorAt : matchStarts[index] >= anchorAt) {
                            return index;
                        }
                    }
                    return 0;
                }
                if (previous >= 0) {
                    if (step === "next") {
                        return (previous + 1) % count;
                    }
                    if (step === "previous") {
                        return (previous + count - 1) % count;
                    }
                    return previous < count ? previous : count - 1;
                }
                if (step === "previous") {
                    return count - 1;
                }
                return firstInView();
            }

            // The first match whose element is not above the viewport (matches are in document
            // order, which is close enough to top-to-bottom for a binary search).
            function firstInView() {
                var low = 0;
                var high = count;
                while (low < high) {
                    var middle = (low + high) >> 1;
                    var node = nodes[nodeAt(starts, nodeCount, matchStarts[middle])];
                    var parent = apply(parentNode, node, []);
                    var below = true;
                    if (parent !== null && apply(nodeType, parent, []) === 1) {
                        below = apply(getBoundingClientRect, parent, []).bottom > 0;
                    }
                    if (below) {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                return low < count ? low : 0;
            }
        }

        // Scrolls the match's element to the middle of the view, unless it is already in view.
        function reveal(textNode) {
            var element = apply(parentNode, textNode, []);
            if (element === null || apply(nodeType, element, []) !== 1) {
                return;
            }
            var rect = apply(getBoundingClientRect, element, []);
            var height = apply(clientHeight, root, []);
            var width = apply(clientWidth, root, []);
            if (rect.top >= 0 && rect.bottom <= height && rect.left >= 0 && rect.right <= width) {
                return;
            }
            apply(scrollIntoView, element, [dictionary("block", "center", "inline", "nearest")]);
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
