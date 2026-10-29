import { describe, expect, it } from "vitest";
import { detectLanguage, isRtl, messageLanguage, openingText, scriptCounts, shouldOffer } from "./lang";

describe("detectLanguage", () => {
  it("ignores tracking links that outweigh a Hebrew newsletter's text", () => {
    const link = "https://links.example.net/e/web_view/ba1fb8ed-ee2c-4ce9-afd0-ad3f5e6b7c8d/trailer?a=e&de=fba";
    const text = ["חופשה משפחתית בגליל: חבילות חדשות לחגים במלונות בצפון", ...Array(12).fill(link), "אם אינך רואה מייל זה לחץ כאן"].join("\n");
    expect(detectLanguage(text).language).toBe("Hebrew");
  });

  it("detects Hebrew even with English names, tickers and signatures mixed in", () => {
    const text = `סיכום פגישה עם המשקיעים
קני שלום,
מצרף סיכום קצר של הפגישה מאתמול עם Orion Technologies (NASDAQ: ORTX):
1. המשקיעים מרוצים מהתוצאות הרבעוניות.
Best regards, Yael Friedman | Tamar Investments | yael@tamar.example`;
    expect(detectLanguage(text).language).toBe("Hebrew");
  });

  it("detects other scripts", () => {
    expect(detectLanguage("مرحبا بكم في اجتماع المستثمرين السنوي").language).toBe("Arabic");
    expect(detectLanguage("Добрый день, прилагаю отчёт за квартал").language).toBe("Russian");
    expect(detectLanguage("会议将于下周三在特拉维夫举行").language).toBe("Chinese");
    expect(detectLanguage("会議は来週の水曜日にテルアビブで行われます").language).toBe("Japanese");
    expect(detectLanguage("다음 주 수요일에 회의가 있습니다").language).toBe("Korean");
  });

  it("tells Latin-script languages apart by common words", () => {
    expect(detectLanguage("Thanks for the call today. Please find the deck attached, and let me know if you have any questions.").language).toBe("English");
    expect(detectLanguage("Bonjour, merci pour votre message. Nous serons à Paris pour la conférence et je vous propose un rendez-vous.").language).toBe("French");
    expect(detectLanguage("Sehr geehrter Herr Green, vielen Dank für die Einladung. Wir freuen uns auf das Treffen mit Ihnen und dem Team.").language).toBe("German");
    expect(detectLanguage("Hola, muchas gracias por la información. Nos vemos en la conferencia de Madrid para hablar de los resultados.").language).toBe("Spanish");
  });

  it("defaults unclear Latin text to English and gives up on too little text", () => {
    expect(detectLanguage("Q3 2026 ORTX EPS beat consensus").language).toBe("English");
    expect(detectLanguage("OK").language).toBe(null);
    expect(detectLanguage("12:30 — 14:00").language).toBe(null);
  });

  it("a bare English signature isn't Italian (initials and abbreviations aren't words)", () => {
    const sig = "Regards\nTom\n\nTom Bergman\nManaging Partner\nJ.R. Harbor Advisory\nMain : 1 212 555 0140\nUS Cell : 1 310 555 0141\nIL Cell : +972 54 555 0142";
    expect(messageLanguage(sig).language).toBe("English");
    expect(detectLanguage("Ciao Tom, grazie per il messaggio. Ci vediamo a Milano per la conferenza e per il pranzo.").language).toBe("Italian");
  });

  it("an English message quoting a little Hebrew stays English", () => {
    const t = "Hi Kenny, see the summary below from the meeting yesterday, and the agenda for next week's roadshow in London. Regards, Jonathan\n> שלום קני";
    expect(detectLanguage(t).language).toBe("English");
  });
});

describe("messageLanguage", () => {
  // Avi's reply: two Hebrew lines over an English signature, disclaimer and a quoted mail
  const reply = [
    "היי מיכל, אני מוסיף את קני שיעזור לך עם הזמנת חדר הישיבות לכנס. הוא מכיר את המקום ויודע מה צריך להזמין מראש.",
    "קני, תוכל לעזור?",
    "",
    "תודה",
    "",
    "Avi Rosen",
    "VP Operations",
    "m: +972-54-555-0166 | t: +972-3-555-0167 |",
    "E-mail: avi.rosen@corvina.example",
    "Website: www.corvina.example",
    "This e-mail and any attachments are confidential and intended only for the addressee at Corvina Medical Ltd.",
    "If you are not the intended recipient, please do not read, copy or forward it.",
    "Please tell the sender and delete the message and any copies of it.",
    "Any views or opinions in this message are those of the author and not necessarily those of the company.",
    "E-mail can be intercepted or changed, so the company accepts no liability for its content once it has been sent.",
    "From: Michal Dor <michal.dor@corvina.example>",
    "Subject: חדר ישיבות לכנס השנתי",
    "היי אבי, מה נשמע? צריך לסגור חדר ישיבות לכנס של החודש הבא.",
    "Michal Dor",
    "Events Coordinator",
  ].join("\n");
  it("goes by the message's opening, not the English signature and disclaimer below it", () => {
    expect(detectLanguage(reply).language).toBe("English"); // what the whole text says
    expect(messageLanguage(reply, "RE: חדר ישיבות לכנס השנתי").language).toBe("Hebrew");
  });
  it("an English message above a Hebrew thread is English", () => {
    expect(messageLanguage("Thanks Avi, I'll book the room for Michal today.\n\nKenny\n\nFrom: Avi\nהיי קני, תוכל לעזור?").language).toBe("English");
  });
  it("skips a quoted header at the top (the original in a reply)", () => {
    expect(openingText("-----Original Message-----\nFrom: Michal\nSent: Monday\nTo: Avi\nSubject: Room\n\nהיי אבי, מה נשמע?")).toBe("היי אבי, מה נשמע?");
    expect(messageLanguage("-----Original Message-----\nFrom: Michal <michal.dor@corvina.example>\nSubject: Room\n\nהיי אבי, מה נשמע? צריך לסגור חדר לכנס").language).toBe(
      "Hebrew",
    );
  });
  it("a one-word forward above a signature logo embedded as data isn't Dutch", () => {
    const logo = `[data:image/png;base64,${"iVBORw0KGgoAAAANSUhEUgQUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0NTY3ODk".repeat(40)}]`;
    const text = `FYI\n\n${logo}\nAvi Rosen\nVP Operations\nThis e-mail is confidential and intended only for the addressee at Corvina Medical Ltd.`;
    expect(messageLanguage(text, "FW: Sector notes for the quarter").language).toBe("English");
  });
  it("too little to tell from the opening: the whole text decides", () => {
    expect(messageLanguage("👍\n\nשלום לכולם, הנה הסיכום של הפגישה מאתמול").language).toBe("Hebrew");
  });
});

describe("helpers", () => {
  it("counts letters per script", () => {
    const c = scriptCounts("abc שלום 1 2");
    expect(c.latin).toBe(3);
    expect(c.hebrew).toBe(4);
  });
  it("knows RTL languages", () => {
    expect(isRtl("Hebrew")).toBe(true);
    expect(isRtl("Arabic")).toBe(true);
    expect(isRtl("English")).toBe(false);
    expect(isRtl(null)).toBe(false);
  });
  it("offers translation only for another, not-excluded language", () => {
    expect(shouldOffer("Hebrew", "English", [])).toBe(true);
    expect(shouldOffer("English", "English", [])).toBe(false);
    expect(shouldOffer("Hebrew", "english", [])).toBe(true);
    expect(shouldOffer("Hebrew", "English", ["hebrew"])).toBe(false);
    expect(shouldOffer(null, "English", [])).toBe(false);
    expect(shouldOffer("English", "Hebrew", [])).toBe(true);
  });
});
