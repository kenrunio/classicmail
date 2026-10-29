import type { FC } from "react";
import type { MenuItem } from "../ui";

export type ModuleId = "mail" | "calendar" | "contacts" | "tasks";

/**
 * What a module plugs into the main window. Each module exports one of these from
 * src/modules/<id>/index.tsx.
 */
export interface ModuleDef {
  id: ModuleId;
  label: string;
  icon: string;
  /** top part of the Navigation Pane (folder tree, date navigator, view list...) */
  NavPane: FC;
  /** the module's main view */
  View: FC;
  /** the Standard toolbar contents while this module is active */
  Toolbar: FC;
  /** menu contributions while active */
  useMenus?: () => ModuleMenus;
  /** Ctrl+N / File › New default item */
  newItem: () => void;
}

export interface ModuleMenus {
  /** replaces the Edit menu body */
  edit?: MenuItem[];
  /** inserted at the top of the View menu (Current View, Arrange By...) */
  view?: MenuItem[];
  /** the Actions menu body */
  actions?: MenuItem[];
}
