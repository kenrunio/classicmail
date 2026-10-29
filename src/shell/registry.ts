// The four modules of the main window (integrator-owned list).
import { mailModule } from "../modules/mail";
import { calendarModule } from "../modules/calendar";
import { contactsModule } from "../modules/contacts";
import { tasksModule } from "../modules/tasks";
import type { ModuleDef, ModuleId } from "./types";

export const MODULES: ModuleDef[] = [mailModule, calendarModule, contactsModule, tasksModule];

export function moduleById(id: ModuleId): ModuleDef {
  return MODULES.find((m) => m.id === id) ?? mailModule;
}
