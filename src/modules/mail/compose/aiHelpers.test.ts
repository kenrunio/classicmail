import { describe, expect, it } from "vitest";
import { canToggle, instructions, showsReplyAll, signatureStart, splitSubject, toggleReply, type ReplyTargets } from "./aiHelpers";

const a = (email: string, name = "") => ({ name, email });
const T: ReplyTargets = {
  one: { to: [a("sarah@harbor.example", "Sarah")], cc: [] },
  all: { to: [a("sarah@harbor.example", "Sarah"), a("david@meridian.example", "David")], cc: [a("info@duvz.demo")] },
};
const emails = (v: { email: string }[]) => v.map((x) => x.email);

describe("Reply to One / Reply All", () => {
  it("cuts down to the sender and back, keeping names the user added", () => {
    const all = { to: [a("sarah@harbor.example", "Sarah"), a("david@meridian.example", "David")], cc: [a("info@duvz.demo"), a("added@x.com", "Added")] };
    const one = toggleReply(all, T, true, (x) => x);
    expect(emails(one.to)).toEqual(["sarah@harbor.example"]);
    expect(emails(one.cc)).toEqual(["added@x.com"]);
    const back = toggleReply(one, T, false, (x) => x);
    expect(emails(back.to)).toEqual(["sarah@harbor.example", "david@meridian.example"]);
    expect(emails(back.cc)).toEqual(["info@duvz.demo", "added@x.com"]);
  });

  it("never lists an address twice", () => {
    const cur = { to: [a("SARAH@harbor.example", "Sarah C")], cc: [a("sarah@harbor.example")] };
    const r = toggleReply(cur, T, true, (x) => x);
    expect(r.to).toEqual([a("SARAH@harbor.example", "Sarah C")]);
    expect(r.cc).toEqual([]);
  });

  it("knows when there is something to toggle and which label to show", () => {
    expect(canToggle(T)).toBe(true);
    expect(canToggle({ one: T.one, all: T.one })).toBe(false);
    expect(canToggle(null)).toBe(false);
    expect(showsReplyAll(T.all)).toBe(true);
    expect(showsReplyAll(T.one)).toBe(false);
  });
});

describe("instructions", () => {
  it("finds { } instructions; [ ] is ordinary text", () => {
    expect(instructions("Dear {first names}, [EXTERNAL] {thank him} {?} {} x")).toEqual(["first names", "thank him"]);
    expect(instructions("no {unclosed\n} here")).toEqual([]);
  });
  it("leaves mail-merge {fields} alone", () => {
    expect(instructions("Dear {First Name}, {thank him}", ["first_name"])).toEqual(["thank him"]);
  });
});

describe("splitSubject", () => {
  it("keeps the reply / forward markers out of the translation", () => {
    expect(splitSubject("RE: FW: Q3 call")).toEqual({ prefix: "RE: FW: ", rest: "Q3 call" });
    expect(splitSubject("השב: פגישה")).toEqual({ prefix: "השב: ", rest: "פגישה" });
    expect(splitSubject("Results: Q3")).toEqual({ prefix: "", rest: "Results: Q3" });
    expect(splitSubject("")).toEqual({ prefix: "", rest: "" });
  });
});

describe("signatureStart", () => {
  const sig = "Kenny Green\nDuvz Software";
  it("finds the trailing signature blocks", () => {
    expect(signatureStart(["Hi Dan,", "Thanks.", "", "Kenny Green", "Duvz Software"], sig)).toBe(3);
  });
  it("is null without a signature or when the user typed below it", () => {
    expect(signatureStart(["Hi Dan,", "Thanks."], sig)).toBeNull();
    expect(signatureStart(["Hi", "Kenny Green", "Duvz Software", "PS more"], sig)).toBeNull();
    expect(signatureStart(["Hi"], "")).toBeNull();
  });
});
