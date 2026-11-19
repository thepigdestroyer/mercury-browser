// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Fills a saved login into the password field the user right-clicked, in the top-level document
// only. Run once through the engine's script evaluation after the user chose the login in the
// browser's own menu; the browser replaces the three arguments. Returns a short status word.
(function (origin, username, password) {
  "use strict";
  if (window.top !== window || location.origin !== origin) {
    return "wrong-origin";
  }
  // Focus inside a frame makes the frame element, not a field, this document's active element.
  var field = document.activeElement;
  if (!(field instanceof HTMLInputElement) || field.type !== "password" || !field.isConnected) {
    return "no-password-field";
  }
  if (field.disabled || field.readOnly) {
    return "read-only";
  }
  var setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  function fill(input, value) {
    setValue.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }
  // The user-name partner: the last editable, rendered text-like field before the password field
  // in its form (or, outside a form, in the document). Hidden ones (honeypots) are skipped.
  var partner = null;
  if (username !== "") {
    var scope = field.form ? field.form.elements : document.querySelectorAll("input");
    var textTypes = ["text", "email", "tel", "url"];
    for (var i = 0; i < scope.length; i++) {
      var input = scope[i];
      if (input === field) {
        break;
      }
      if (input instanceof HTMLInputElement && textTypes.indexOf(input.type) !== -1 &&
          !input.disabled && !input.readOnly && input.getClientRects().length > 0) {
        partner = input;
      }
    }
  }
  if (partner !== null) {
    fill(partner, username);
  }
  fill(field, password);
  return partner !== null ? "filled" : "filled-password";
})
