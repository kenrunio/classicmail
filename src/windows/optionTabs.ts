// Extra Options tabs contributed by modules (Mail Format / signatures, AI). Integrator-owned list.
import type { FC } from "react";
import { MailFormatOptionsTab } from "../modules/mail/compose/SignaturesDialog";
import { AiSettings } from "../modules/ai/AiSettings";
import { LargeFilesOptions } from "../modules/mail/compose/LargeFilesOptions";

export const OPTION_EXTRA_TABS: { id: string; label: string; Component: FC }[] = [
  { id: "mailformat", label: "Mail Format", Component: MailFormatOptionsTab },
  { id: "largefiles", label: "Large Files", Component: LargeFilesOptions },
  { id: "ai", label: "AI", Component: AiSettings },
];
