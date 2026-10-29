import { describe, expect, it } from "vitest";
import { firstUrl, linkParts } from "./links";

describe("linkParts", () => {
  it("finds web addresses, leaving trailing punctuation and Outlook's <…> as text", () => {
    expect(linkParts("Join: https://zoom.us/j/1?pwd=x. Then www.example.com, or <https://a.b/c>")).toEqual([
      { text: "Join: " },
      { text: "https://zoom.us/j/1?pwd=x", url: "https://zoom.us/j/1?pwd=x" },
      { text: ". Then " },
      { text: "www.example.com", url: "https://www.example.com" },
      { text: ", or <" },
      { text: "https://a.b/c", url: "https://a.b/c" },
      { text: ">" },
    ]);
    expect(linkParts("no links here")).toEqual([{ text: "no links here" }]);
  });
  it("picks a Location's link", () => {
    expect(firstUrl("https://zoom.us/j/8125550147?pwd=dEx4")).toBe("https://zoom.us/j/8125550147?pwd=dEx4");
    expect(firstUrl("Room 12")).toBeUndefined();
  });
});
