import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, parseColors, splitSnippet } from "./format";

describe("formatBytes", () => {
  it("picks sensible units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(20 * 1024 * 1024)).toBe("20 MB");
    expect(formatBytes(3 * 1024 ** 4)).toBe("3.0 TB");
  });
});

describe("formatDuration", () => {
  it("formats mm:ss and h:mm:ss", () => {
    expect(formatDuration(61_000)).toBe("1:01");
    expect(formatDuration(3_725_000)).toBe("1:02:05");
    expect(formatDuration(null)).toBe("");
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

describe("parseColors", () => {
  it("parses the stored JSON and tolerates garbage", () => {
    expect(parseColors("[[8,8,248,0.9],[0,0,0,0.1]]")).toEqual([
      { rgb: "rgb(8,8,248)", share: 0.9 },
      { rgb: "rgb(0,0,0)", share: 0.1 },
    ]);
    expect(parseColors("nope")).toEqual([]);
    expect(parseColors(null)).toEqual([]);
  });
});
