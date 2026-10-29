// Local, offline language detection for the Translate bar: script ratios for non-Latin scripts
// (Hebrew, Arabic, Cyrillic, Greek, Thai, CJK) and common-word scores for a few Latin-script
// languages. Good enough to decide "is this message mostly in another language?" — nothing is
// sent anywhere to find out.

export const TARGET_LANGUAGES = ["English", "Hebrew", "Arabic", "Russian", "French", "German", "Spanish", "Italian", "Portuguese", "Dutch", "Chinese", "Japanese"] as const;

const RTL = new Set(["Hebrew", "Arabic", "Persian"]);
export const isRtl = (lang: string | null | undefined) => !!lang && RTL.has(lang);

type Script = "latin" | "hebrew" | "arabic" | "cyrillic" | "greek" | "thai" | "han" | "kana" | "hangul";

function scriptOf(cp: number): Script | null {
  if ((cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a) || (cp >= 0xc0 && cp <= 0x24f && cp !== 0xd7 && cp !== 0xf7)) return "latin";
  if (cp >= 0x5d0 && cp <= 0x5ea) return "hebrew";
  if ((cp >= 0x600 && cp <= 0x6ff) || (cp >= 0x750 && cp <= 0x77f) || (cp >= 0xfb50 && cp <= 0xfdff) || (cp >= 0xfe70 && cp <= 0xfeff)) return "arabic";
  if (cp >= 0x400 && cp <= 0x4ff) return "cyrillic";
  if (cp >= 0x370 && cp <= 0x3ff) return "greek";
  if (cp >= 0xe00 && cp <= 0xe7f) return "thai";
  if ((cp >= 0x3040 && cp <= 0x30ff) || (cp >= 0x31f0 && cp <= 0x31ff)) return "kana";
  if (cp >= 0xac00 && cp <= 0xd7af) return "hangul";
  if ((cp >= 0x4e00 && cp <= 0x9fff) || (cp >= 0x3400 && cp <= 0x4dbf)) return "han";
  return null;
}

/** Letters per script. */
export function scriptCounts(text: string): Record<Script, number> {
  const c: Record<Script, number> = { latin: 0, hebrew: 0, arabic: 0, cyrillic: 0, greek: 0, thai: 0, han: 0, kana: 0, hangul: 0 };
  for (const ch of text) {
    const s = scriptOf(ch.codePointAt(0)!);
    if (s) c[s]++;
  }
  return c;
}

// Most frequent function words; a language "wins" by share of matching tokens.
const STOPWORDS: Record<string, string[]> = {
  English: [
    "the",
    "and",
    "to",
    "of",
    "a",
    "in",
    "is",
    "you",
    "that",
    "for",
    "it",
    "on",
    "with",
    "we",
    "be",
    "are",
    "this",
    "have",
    "as",
    "will",
    "your",
    "at",
    "our",
    "please",
    "thanks",
    "regards",
    "best",
    "i",
    "not",
    "or",
    "from",
    "by",
  ],
  French: [
    "le",
    "la",
    "les",
    "de",
    "des",
    "et",
    "un",
    "une",
    "est",
    "pour",
    "que",
    "qui",
    "dans",
    "vous",
    "nous",
    "pas",
    "sur",
    "avec",
    "du",
    "au",
    "merci",
    "bonjour",
    "cordialement",
    "je",
    "il",
    "ce",
  ],
  German: [
    "der",
    "die",
    "das",
    "und",
    "ist",
    "nicht",
    "mit",
    "sie",
    "ich",
    "wir",
    "ein",
    "eine",
    "zu",
    "von",
    "den",
    "dem",
    "auf",
    "für",
    "auch",
    "es",
    "im",
    "bitte",
    "danke",
    "grüße",
    "gruß",
    "freundlichen",
  ],
  Spanish: [
    "el",
    "la",
    "los",
    "las",
    "de",
    "y",
    "que",
    "en",
    "un",
    "una",
    "es",
    "por",
    "para",
    "con",
    "no",
    "se",
    "su",
    "del",
    "al",
    "muy",
    "gracias",
    "saludos",
    "hola",
    "usted",
    "nos",
    "lo",
  ],
  Italian: [
    "il",
    "lo",
    "la",
    "gli",
    "le",
    "di",
    "e",
    "che",
    "un",
    "una",
    "è",
    "per",
    "con",
    "non",
    "sono",
    "del",
    "della",
    "al",
    "grazie",
    "saluti",
    "cordiali",
    "ciao",
    "ci",
    "si",
    "mi",
  ],
  Portuguese: [
    "o",
    "a",
    "os",
    "as",
    "de",
    "e",
    "que",
    "em",
    "um",
    "uma",
    "é",
    "para",
    "com",
    "não",
    "do",
    "da",
    "dos",
    "no",
    "na",
    "obrigado",
    "obrigada",
    "atenciosamente",
    "você",
    "nós",
    "se",
  ],
  Dutch: [
    "de",
    "het",
    "een",
    "en",
    "van",
    "is",
    "dat",
    "niet",
    "met",
    "voor",
    "op",
    "zijn",
    "wij",
    "we",
    "ik",
    "je",
    "u",
    "aan",
    "ook",
    "bedankt",
    "groet",
    "groeten",
    "vriendelijke",
    "dank",
  ],
};
const STOP_SETS = Object.fromEntries(Object.entries(STOPWORDS).map(([k, v]) => [k, new Set(v)])) as Record<string, Set<string>>;

function latinLanguage(text: string): string | null {
  const all = text.match(/[a-zA-Zà-öø-ÿÀ-ÖØ-Þ]+/g) ?? [];
  if (all.length < 3) return null;
  // words in capitals are initials and abbreviations, not words of the language: "J.R. Harbor Advisory",
  // "IL Cell" in a signature read as the Italian "e" (and) and "il" (the)
  const words = all.filter((w) => w !== w.toUpperCase()).map((w) => w.toLowerCase());
  let best: string | null = null;
  let bestScore = 0;
  let bestHits = 0;
  for (const [lang, set] of Object.entries(STOP_SETS)) {
    let n = 0;
    for (const w of words) if (set.has(w)) n++;
    const score = words.length ? n / words.length : 0;
    if (score > bestScore) {
      bestScore = score;
      best = lang;
      bestHits = n;
    }
  }
  // English is the safe default for Latin text without a clear signal (names, numbers, jargon,
  // a bare signature): another language needs at least two of its common words
  return bestScore >= 0.08 && (best === "English" || bestHits >= 2) ? best : "English";
}

/**
 * Links, e-mail addresses, pictures embedded as data (a signature logo's "data:image/png;base64,…")
 * and code-like tokens (tracking ids, hex) say nothing about the language and can outweigh the real
 * text: a Hebrew newsletter's text part is often mostly tracking URLs, whose letter soup ("a", "e",
 * "de"…) then reads as Portuguese, and a logo's base64 under a one-word "FYI" read as Dutch.
 */
export function stripNonWords(text: string): string {
  return text
    .replace(/data:[\w/+.-]+;base64,[A-Za-z0-9+/=\s]*/g, " ")
    .replace(/[A-Za-z0-9+/=]{40,}/g, " ")
    .replace(/\b(?:https?:\/\/|www\.)\S+/gi, " ")
    .replace(/\S+@\S+\.\S+/g, " ")
    .replace(/[\w-]*\d[\w-]*/g, " ");
}

export interface Detected {
  language: string | null;
  /** share of letters in the detected language's script (0..1) */
  share: number;
}

/**
 * The main language of `text`. A non-Latin script wins when it is at least 35% of the letters
 * (Hebrew mail is full of English names, tickers and signatures).
 */
export function detectLanguage(text: string): Detected {
  const sample = stripNonWords(text.length > 20000 ? text.slice(0, 20000) : text);
  const c = scriptCounts(sample);
  const total = Object.values(c).reduce((a, b) => a + b, 0);
  if (total < 4) return { language: null, share: 0 };
  const cjk = c.han + c.kana + c.hangul;
  const candidates: [string, number][] = [
    ["Hebrew", c.hebrew],
    ["Arabic", c.arabic],
    ["Russian", c.cyrillic],
    ["Greek", c.greek],
    ["Thai", c.thai],
    [c.hangul > c.kana && c.hangul >= c.han ? "Korean" : c.kana > 0 ? "Japanese" : "Chinese", cjk],
  ];
  candidates.sort((a, b) => b[1] - a[1]);
  const [lang, n] = candidates[0];
  if (n / total >= 0.35) return { language: lang, share: n / total };
  if (c.latin / total >= 0.5) return { language: latinLanguage(sample), share: c.latin / total };
  return { language: null, share: 0 };
}

/** Lines of a quoted message's header ("-----Original Message-----", "From: …", "Subject: …"). */
const HEADER_LINE = /^\s*(-{2,}\s*original message\s*-{2,}|-{2,}\s*forwarded message\s*-{2,}|(from|sent|date|to|cc|subject)\s*:)/i;

/** The opening of a message: its first lines with words, up to about `letters` letters, after a
 *  quoted header if the text starts with one (a reply's original, a forward). */
export function openingText(text: string, letters = 300): string {
  const lines = text.split(/\r?\n/);
  let i = 0;
  // a quoted header at the top: skip it, through its Subject line
  if (lines.find((l) => l.trim()) && HEADER_LINE.test(lines.find((l) => l.trim())!)) {
    while (i < lines.length && (!lines[i].trim() || HEADER_LINE.test(lines[i]))) i++;
  }
  const out: string[] = [];
  let n = 0;
  for (; i < lines.length && n < letters; i++) {
    // lines with no words (a signature logo embedded as data, a bare link) aren't the opening
    if (!/\p{L}/u.test(stripNonWords(lines[i]))) continue;
    out.push(lines[i]);
    n += (lines[i].match(/\p{L}/gu) ?? []).length;
  }
  return out.join("\n");
}

/** The language a message is written in, as a reader sees it: its opening words. Signatures,
 *  legal disclaimers and the quoted history below are often in another language (English under a
 *  Hebrew e-mail) and outweigh a short message, so they only count when the opening says nothing. */
export function messageLanguage(text: string, subject = ""): Detected {
  const opening = detectLanguage(openingText(text));
  if (opening.language) return opening;
  return detectLanguage(`${subject}\n${text}`);
}

/** Should the Translate bar offer a translation of a message in `detected` into `target`? */
export function shouldOffer(detected: string | null, target: string, never: string[]): boolean {
  if (!detected) return false;
  if (detected.toLowerCase() === target.toLowerCase()) return false;
  return !never.some((n) => n.toLowerCase() === detected.toLowerCase());
}
