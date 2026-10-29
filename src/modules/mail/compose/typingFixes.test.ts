import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { rpc } from "../../../api/rpc";
import { attachTypingFixes, capsFix, fromHebrewLayout, listMarker, urlBefore, wordBefore } from "./typingFixes";

vi.mock("../../../api/rpc", () => ({ rpc: vi.fn(() => Promise.resolve(null)) }));

describe("capsFix", () => {
  it("fixes a word typed with Caps Lock left on", () => {
    expect(capsFix("eLI")).toBe("Eli");
    expect(capsFix("pLEASE")).toBe("Please");
  });
  it("leaves real words and acronyms alone", () => {
    for (const w of ["eL", "IR", "Eli", "iPhone", "eBay", "NASDAQ", "please"]) expect(capsFix(w)).toBeNull();
  });
});

describe("fromHebrewLayout", () => {
  it("turns English typed on the Hebrew keyboard back into English", () => {
    expect(fromHebrewLayout("Eךן")).toBe("Eli");
    expect(fromHebrewLayout("Pךקשדק")).toBe("Please");
    expect(fromHebrewLayout("Kקממט")).toBe("Kenny");
    expect(fromHebrewLayout("Wקךך")).toBe("Well");
    expect(fromHebrewLayout("Sםם'")).toBe("Soow");
    expect(fromHebrewLayout("Dםמ,א")).toBe("Don't");
  });
  it("leaves Hebrew, English and mixed words alone", () => {
    for (const w of ["שלום", "Eli", "IRים", "Eך", "הIR", "eךן"]) expect(fromHebrewLayout(w)).toBeNull();
  });
});

describe("wordBefore", () => {
  it("is the last word, with the Hebrew-letter punctuation keys", () => {
    expect(wordBefore("Hi Eךן")).toBe("Eךן");
    expect(wordBefore("x W'ךך")).toBe("W'ךך");
    expect(wordBefore("end. ")).toBe("");
  });
});

describe("urlBefore", () => {
  it("finds a web address just typed", () => {
    expect(urlBefore("see www.example.com")).toEqual({ url: "www.example.com", href: "http://www.example.com", trail: 0 });
    expect(urlBefore("https://x.com/a?b=1")).toEqual({ url: "https://x.com/a?b=1", href: "https://x.com/a?b=1", trail: 0 });
    expect(urlBefore("(at www.x.com).")).toEqual({ url: "www.x.com", href: "http://www.x.com", trail: 2 });
  });
  it("takes a zero-width space as a word break", () => {
    expect(urlBefore("\u200bwww.x.com")).toEqual({ url: "www.x.com", href: "http://www.x.com", trail: 0 });
  });
  it("leaves other words alone", () => {
    for (const t of ["hello", "awww.x.com", "www.", "https://", "mail eli@x.com", ""]) expect(urlBefore(t)).toBeNull();
  });
});

describe("listMarker", () => {
  it("knows the list markers", () => {
    expect(listMarker("-")).toBe("insertUnorderedList");
    expect(listMarker("*")).toBe("insertUnorderedList");
    expect(listMarker("\u200b1.")).toBe("insertOrderedList");
  });
  it("leaves other line starts alone", () => {
    for (const t of ["", "--", "2.", "1", "a-", " -", "1.)"]) expect(listMarker(t)).toBeNull();
  });
});

describe("attachTypingFixes: links and lists on Space (execCommand stubbed: jsdom has none)", () => {
  let root: HTMLElement;
  let detach: () => void;
  const calls: string[] = [];
  const realExec = document.execCommand;
  const typeSpaceAtEnd = (el: Element) => {
    const sel = document.getSelection()!;
    const r = document.createRange();
    r.selectNodeContents(el);
    r.collapse(false);
    sel.removeAllRanges();
    sel.addRange(r);
    return el.dispatchEvent(new KeyboardEvent("keydown", { key: " ", code: "Space", bubbles: true, cancelable: true }));
  };

  beforeEach(() => {
    calls.length = 0;
    document.execCommand = vi.fn((cmd: string, _ui?: boolean, arg?: string) => {
      calls.push(cmd);
      const r = document.getSelection()!.getRangeAt(0);
      if (cmd === "createLink") {
        const a = document.createElement("a");
        a.setAttribute("href", arg!);
        a.append(r.extractContents());
        r.insertNode(a);
      }
      if (cmd === "delete") r.deleteContents();
      return true;
    });
    document.body.innerHTML = '<div id="root" contenteditable="true"><p id="a"></p><ul><li id="li">-</li></ul><div id="cm-original"><p id="o">-</p></div></div>';
    root = document.getElementById("root")!;
    detach = attachTypingFixes(root, { replyOnly: (b) => !root.querySelector("#cm-original")?.contains(b) });
  });
  afterEach(() => {
    detach();
    document.body.innerHTML = "";
    document.execCommand = realExec;
  });

  it("links exactly the address, punctuation outside", () => {
    const p = document.getElementById("a")!;
    for (const [typed, before, after] of [
      ["visit www.x.com.", "visit ", "."],
      ["(see www.x.com)", "(see ", ")"],
    ]) {
      p.textContent = typed;
      typeSpaceAtEnd(p);
      const a = p.querySelector("a")!;
      expect(a.textContent).toBe("www.x.com");
      expect(a.getAttribute("href")).toBe("http://www.x.com");
      expect(p.textContent).toBe(typed);
      expect(a.previousSibling?.textContent).toBe(before);
      expect(a.nextSibling?.textContent).toBe(after);
      // the caret stays after the punctuation
      expect(document.getSelection()!.toString()).toBe("");
    }
  });
  it("turns '- ' at a reply line's start into a list, not in a list item or the original", () => {
    const p = document.getElementById("a")!;
    p.textContent = "-";
    expect(typeSpaceAtEnd(p)).toBe(false); // the space is not typed
    expect(calls).toEqual(["delete", "insertUnorderedList"]);
    expect(p.textContent).toBe("");
    calls.length = 0;
    expect(typeSpaceAtEnd(document.getElementById("li")!)).toBe(true);
    expect(typeSpaceAtEnd(document.getElementById("o")!)).toBe(true);
    expect(calls).toEqual([]);
  });
});

describe("attachTypingFixes: direction", () => {
  let root: HTMLElement;
  let detach: () => void;
  const caretIn = (el: Element) => {
    const sel = document.getSelection()!;
    sel.removeAllRanges();
    const r = document.createRange();
    r.setStart(el, 0);
    r.collapse(true);
    sel.addRange(r);
  };
  const keyDown = (el: Element, code: string, key: string, init: KeyboardEventInit = {}) => {
    caretIn(el);
    el.dispatchEvent(new KeyboardEvent("keydown", { code, key, bubbles: true, ...init }));
  };

  beforeEach(() => {
    document.body.innerHTML = '<div id="root" contenteditable="true"><p id="reply"></p><p id="reply2">text</p><div id="cm-original"><p id="orig"></p></div></div>';
    root = document.getElementById("root")!;
    detach = attachTypingFixes(root, { replyOnly: (b) => !root.querySelector("#cm-original")?.contains(b) });
  });
  afterEach(() => {
    detach();
    document.body.innerHTML = "";
  });

  it("direction follows the keyboard in the reply only", () => {
    const reply = document.getElementById("reply")!;
    const orig = document.getElementById("orig")!;
    keyDown(reply, "KeyA", "ש");
    expect(reply.getAttribute("dir")).toBe("rtl");
    keyDown(orig, "KeyA", "ש");
    expect(orig.hasAttribute("dir")).toBe(false);
  });

  it("an English key turns an empty reply paragraph back to left-to-right", () => {
    const reply = document.getElementById("reply")!;
    keyDown(reply, "KeyA", "ש");
    keyDown(reply, "KeyA", "a");
    expect(reply.getAttribute("dir")).toBe("ltr");
  });

  it("Shift, Ctrl and Caps Lock keys do not tell the keyboard", () => {
    const reply = document.getElementById("reply")!;
    keyDown(reply, "KeyA", "ש");
    keyDown(reply, "KeyA", "A", { shiftKey: true });
    keyDown(reply, "KeyA", "a", { ctrlKey: true });
    expect(reply.getAttribute("dir")).toBe("rtl");
  });

  it("a paragraph that already has text keeps its direction", () => {
    const reply2 = document.getElementById("reply2")!;
    keyDown(reply2, "KeyA", "ש");
    expect(reply2.hasAttribute("dir")).toBe(false);
  });

  it("an empty line that Ctrl+Z / Ctrl+Y left keeps its direction (the undo stays exact); other input sets it", () => {
    const reply = document.getElementById("reply")!;
    const reply2 = document.getElementById("reply2")!;
    keyDown(reply2, "KeyA", "a"); // the keyboard is English now
    reply.setAttribute("dir", "auto");
    caretIn(reply);
    for (const inputType of ["historyUndo", "historyRedo"]) reply.dispatchEvent(new InputEvent("input", { inputType, bubbles: true }));
    expect(reply.getAttribute("dir")).toBe("auto");
    reply.dispatchEvent(new InputEvent("input", { inputType: "deleteContentBackward", bubbles: true }));
    expect(reply.getAttribute("dir")).toBe("ltr");
  });

  it("the keyboard poll sets an empty line's direction only when the keyboard changed", async () => {
    const reply = document.getElementById("reply")!;
    const reply2 = document.getElementById("reply2")!;
    keyDown(reply2, "KeyA", "a"); // English
    reply.setAttribute("dir", "auto"); // e.g. a line Ctrl+Z just put back
    caretIn(reply);
    const layout = (rtl: boolean) =>
      vi.mocked(rpc).mockImplementation((m: string) => Promise.resolve((m === "app.keyboard.layout" ? { lang: rtl ? "he" : "en", rtl } : null) as never));
    const poll = async () => {
      root.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 0));
    };
    layout(false);
    await poll();
    expect(reply.getAttribute("dir")).toBe("auto");
    layout(true);
    await poll();
    expect(reply.getAttribute("dir")).toBe("rtl");
    vi.mocked(rpc).mockImplementation(() => Promise.resolve(null as never));
  });

  it("detach stops the listeners", () => {
    detach();
    const reply = document.getElementById("reply")!;
    keyDown(reply, "KeyA", "ש");
    expect(reply.hasAttribute("dir")).toBe(false);
  });

  const capsTypo = () => {
    const reply = document.getElementById("reply")!;
    reply.textContent = "hi eL";
    const t = reply.firstChild!;
    keyDown(reply, "KeyI", "I", { shiftKey: true, modifierCapsLock: true });
    const sel = document.getSelection()!;
    sel.collapse(t, 5);
    const ev = new InputEvent("beforeinput", { inputType: "insertText", data: "I", bubbles: true, cancelable: true });
    reply.dispatchEvent(ev);
    return { reply, sel, t, ev };
  };

  it("Caps Lock fix without execCommand leaves the typed letter alone", () => {
    const { reply, ev } = capsTypo();
    expect(document.execCommand).toBeUndefined();
    expect(ev.defaultPrevented).toBe(false);
    expect(reply.textContent).toBe("hi eL");
  });

  it("Caps Lock fix when execCommand refuses puts the caret back and leaves the letter alone", () => {
    (document as unknown as { execCommand: unknown }).execCommand = vi.fn(() => false);
    try {
      const { reply, sel, t, ev } = capsTypo();
      expect(document.execCommand).toHaveBeenCalled();
      expect(ev.defaultPrevented).toBe(false);
      expect(reply.textContent).toBe("hi eL");
      expect(sel.isCollapsed).toBe(true);
      expect(sel.anchorNode).toBe(t);
      expect(sel.anchorOffset).toBe(5);
    } finally {
      delete (document as unknown as { execCommand?: unknown }).execCommand;
    }
  });
});

describe("attachTypingFixes: replacement (execCommand stubbed)", () => {
  let root: HTMLElement;
  let detach: () => void;
  const rpcMock = vi.mocked(rpc);

  beforeEach(() => {
    document.body.innerHTML = '<div id="root" contenteditable="true"><p id="p"></p></div>';
    root = document.getElementById("root")!;
    detach = attachTypingFixes(root, { replyOnly: () => true });
    // jsdom has no execCommand: stand in for insertText by replacing the selected range
    (document as unknown as { execCommand: unknown }).execCommand = vi.fn((cmd: string, _ui: boolean, text: string) => {
      if (cmd !== "insertText") return false;
      const r = document.getSelection()!.getRangeAt(0);
      r.deleteContents();
      r.insertNode(document.createTextNode(text));
      return true;
    });
    rpcMock.mockClear();
  });
  afterEach(() => {
    detach();
    delete (document as unknown as { execCommand?: unknown }).execCommand;
    document.body.innerHTML = "";
  });

  const typeLetter = (p: HTMLElement, text: string, key: string, caps: boolean) => {
    const t = p.lastChild!;
    document.getSelection()!.collapse(t, t.textContent!.length);
    p.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyL", key, shiftKey: true, modifierCapsLock: caps, bubbles: true }));
    const ev = new InputEvent("beforeinput", { inputType: "insertText", data: text, bubbles: true, cancelable: true });
    p.dispatchEvent(ev);
    return ev;
  };

  it("Caps Lock left on: eL + I becomes Eli-style and Caps Lock goes off", () => {
    const p = document.getElementById("p")!;
    p.textContent = "hi eL";
    const ev = typeLetter(p, "I", "I", true);
    expect(ev.defaultPrevented).toBe(true);
    expect(p.textContent).toBe("hi Eli");
    expect(rpcMock).toHaveBeenCalledWith("app.keyboard.capsLockOff", {});
  });

  it("Caps Lock: two letters are not enough", () => {
    const p = document.getElementById("p")!;
    p.textContent = "e";
    const ev = typeLetter(p, "L", "L", true);
    expect(ev.defaultPrevented).toBe(false);
    expect(p.textContent).toBe("e");
  });

  it("Hebrew layout: the word becomes English when a space ends it", () => {
    const p = document.getElementById("p")!;
    p.textContent = "hi Eךן";
    document.getSelection()!.collapse(p.firstChild!, 6);
    const ev = new InputEvent("beforeinput", { inputType: "insertText", data: " ", bubbles: true, cancelable: true });
    p.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    expect(p.textContent).toBe("hi Eli ");
    expect(rpcMock).toHaveBeenCalledWith("app.keyboard.switchToEnglish", {});
  });

  it("Hebrew layout: Enter fixes the word and lets the new line happen", () => {
    const p = document.getElementById("p")!;
    p.innerHTML = "<b>hi </b>Eךן";
    document.getSelection()!.collapse(p.lastChild!, 3);
    const ev = new KeyboardEvent("keydown", { code: "Enter", key: "Enter", bubbles: true, cancelable: true });
    p.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    expect(p.textContent).toBe("hi Eli");
  });

  it("Hebrew layout: a word that spans two formatting runs is replaced whole", () => {
    const p = document.getElementById("p")!;
    p.innerHTML = "E<b>ךן</b>";
    document.getSelection()!.collapse(p.querySelector("b")!.firstChild!, 2);
    p.dispatchEvent(new InputEvent("beforeinput", { inputType: "insertText", data: "?", bubbles: true, cancelable: true }));
    expect(p.textContent).toBe("Eli?");
  });
});
