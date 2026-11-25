/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: (MED-1) carried unchanged from servo/servo#47269 by decomoraes, MPL-2.0 (the interface is
// behind `dom_media_source_enabled`, default off).

// https://w3c.github.io/media-source/#idl-def-sourcebufferlist

[Exposed=Window, Pref="dom_media_source_enabled"]
interface SourceBufferList : EventTarget {
  readonly attribute unsigned long length;

  attribute EventHandler onaddsourcebuffer;
  attribute EventHandler onremovesourcebuffer;

  getter SourceBuffer (unsigned long index);
};
