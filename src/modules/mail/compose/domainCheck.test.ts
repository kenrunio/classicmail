import { describe, expect, it } from "vitest";
import { companiesWarning, domainOf } from "./domainCheck";

const me = ["kenny@duvz.demo", "info@duvz.demo"];

describe("companiesWarning", () => {
  it("warns about a second company, most people first", () => {
    const r = companiesWarning(["a@orbit.example", "b@orbit.example", "c@Orbit.example", "x@random.example"], me);
    expect(r).toEqual([
      { domain: "orbit.example", count: 3 },
      { domain: "random.example", count: 1 },
    ]);
  });

  it("ignores your own domains, subdomains included", () => {
    expect(companiesWarning(["a@orbit.example", "dana@duvz.demo", "ops@mail.duvz.demo"], me)).toBeNull();
  });

  it("is quiet for one company, and counts an address once", () => {
    expect(companiesWarning(["a@orbit.example", "A@orbit.example", "b@orbit.example"], me)).toBeNull();
    expect(companiesWarning([], me)).toBeNull();
  });

  it("a reply warns only when it adds a company that wasn't in the conversation", () => {
    const conv = ["cfo@orbit.example", "analyst@broker.example"];
    expect(companiesWarning(["cfo@orbit.example", "analyst@broker.example", "kenny@duvz.demo"], me, conv)).toBeNull();
    expect(companiesWarning(["cfo@orbit.example", "someone@random.example"], me, conv)).toEqual([
      { domain: "orbit.example", count: 1 },
      { domain: "random.example", count: 1 },
    ]);
  });

  it("domainOf", () => {
    expect(domainOf("Dana@Harbor.Example")).toBe("harbor.example");
    expect(domainOf("no-at-sign")).toBe("");
  });
});
