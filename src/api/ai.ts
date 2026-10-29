// AI DTOs (mirror src-tauri/src/ai/*.rs) and rpc wrappers.
import { rpc } from "./rpc";
import { withAiConsent } from "./aiConsent";

export type AiProvider = "gemini" | "claude";

export interface AiSettingsDto {
  provider: AiProvider;
  /** saved model ("" = provider default) */
  model: string;
  defaultModels: Record<AiProvider, string>;
  effectiveModel: string;
  hasGeminiKey: boolean;
  hasAnthropicKey: boolean;
  /** a key exists for the selected provider (or the demo provider is in use) */
  configured: boolean;
  demo: boolean;
  targetLanguage: string;
  neverTranslate: string[];
  /** when the one-time AI notice was accepted (epoch ms), null = not yet */
  consentAt: number | null;
}

export interface AiSetSettings {
  provider?: AiProvider;
  model?: string;
  /** key for the (new or saved) provider; "" removes it */
  key?: string;
  targetLanguage?: string;
  neverTranslate?: string[];
}

export interface TranslateResult {
  translations: string[];
  target: string;
  cached: number;
  translated: number;
  failed: number;
  demo: boolean;
}

export const aiApi = {
  getSettings: () => rpc<AiSettingsDto>("ai.getSettings"),
  setSettings: (p: AiSetSettings) => rpc<AiSettingsDto>("ai.setSettings", p),
  test: (p: { provider?: AiProvider; model?: string; key?: string }) => rpc<{ ok: boolean; message: string }>("ai.test", p),
  /** models the saved key can use, newest first, with a recommended default */
  /** the models the key can use, newest first; checkedAt: when the provider was asked (epoch ms; also once a day in the background) */
  listModels: (p: { provider?: AiProvider; key?: string } = {}) =>
    rpc<{ models: { id: string; label: string }[]; recommended: string | null; checkedAt: number }>("ai.listModels", p),
  /** accept (true) or withdraw (false) the one-time AI notice */
  setConsent: (consent: boolean) => rpc<AiSettingsDto>("ai.setConsent", { consent }),
  translate: (messageId: number, texts: string[], target?: string) =>
    withAiConsent(() => rpc<TranslateResult>("ai.translate", { messageId, texts, ...(target ? { target } : {}) })),
};
