// AI help while writing (src-tauri/src/ai/compose.rs): translate my reply, Reply with AI, AI Review.
import { rpc } from "./rpc";
import { withAiConsent } from "./aiConsent";

export const aiComposeApi = {
  /** the user's reply in informal e-mail `language` (HTML in, HTML out) */
  translate: (html: string, language: string) => withAiConsent(() => rpc<{ html: string }>("ai.compose.translate", { html, language })),
  /** a short English reply to a message, as HTML paragraphs */
  draftReply: (messageId: number) => withAiConsent(() => rpc<{ html: string }>("ai.compose.draftReply", { messageId })),
  /** carry out the {instructions} in the text (`fields`: mail-merge columns, whose {Field} stay) */
  review: (html: string, context: string, fields: string[]) => withAiConsent(() => rpc<{ html: string; applied: number }>("ai.compose.review", { html, context, fields })),
};
