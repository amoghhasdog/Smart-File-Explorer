import { describe, expect, it } from "vitest";
import { formatBytes, splitSnippet } from "./format";

describe("formatBytes", () => {
  it("picks sensible units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(20 * 1024 * 1024)).toBe("20 MB");
    expect(formatBytes(3 * 1024 ** 4)).toBe("3.0 TB");
  });
});

describe("splitSnippet", () => {
  it("separates highlighted matches", () => {
    expect(splitSnippet("The [Ferrari] won…")).toEqual([
      { text: "The ", hit: false },
      { text: "Ferrari", hit: true },
      { text: " won…", hit: false },
    ]);
    expect(splitSnippet("plain")).toEqual([{ text: "plain", hit: false }]);
    expect(splitSnippet("")).toEqual([]);
  });
});
