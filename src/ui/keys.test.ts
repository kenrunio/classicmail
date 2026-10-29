import { describe, expect, it } from "vitest";
import { keyOf } from "./keys";

describe("keyOf", () => {
  it("reads the key as typed on a Latin layout", () => {
    expect(keyOf({ key: "R", code: "KeyR" })).toBe("r");
    expect(keyOf({ key: "Delete", code: "Delete" })).toBe("delete");
    expect(keyOf({ key: "=", code: "Equal" })).toBe("=");
    // AZERTY: the A key is where Q is on a US keyboard, and Ctrl+A still means A
    expect(keyOf({ key: "a", code: "KeyQ" })).toBe("a");
  });

  it("uses the key's place on the keyboard for other alphabets", () => {
    expect(keyOf({ key: "ר", code: "KeyR" })).toBe("r");
    expect(keyOf({ key: "ט", code: "KeyY" })).toBe("y");
    expect(keyOf({ key: "ץ", code: "Period" })).toBe(".");
  });

  it("reads a digit key as its digit, with Shift or on a French keyboard", () => {
    expect(keyOf({ key: "!", code: "Digit1" })).toBe("1");
    expect(keyOf({ key: "&", code: "Digit1" })).toBe("1");
  });
});
