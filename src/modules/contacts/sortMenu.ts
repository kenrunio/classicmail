// Menus for View › Sort By / Display File As, the pane header's "Sort by" drop-down and the
// right-click menu on the view background.
import { showMenu, type MenuItem } from "../../ui";
import { currentAccountId, currentGroupResource, newContact } from "./actions";
import { SORT_KEYS } from "./sort";
import { VIEWS, useContactsUi } from "./store";

/** First Name … Date Modified, then Ascending / Descending. */
export function sortMenu(): MenuItem[] {
  const { sort, setSort } = useContactsUi.getState();
  return [
    ...SORT_KEYS.map((k): MenuItem => ({ label: k.label, radio: true, checked: sort.key === k.id, onClick: () => setSort(k.id) })),
    { type: "separator" },
    { label: "Ascending", radio: true, checked: !sort.desc, onClick: () => setSort(sort.key, false) },
    { label: "Descending", radio: true, checked: sort.desc, onClick: () => setSort(sort.key, true) },
  ];
}

export function nameOrderMenu(): MenuItem[] {
  const { nameOrder, setNameOrder } = useContactsUi.getState();
  return [
    { label: "Last, First", radio: true, checked: nameOrder === "lastFirst", onClick: () => setNameOrder("lastFirst") },
    { label: "First Last", radio: true, checked: nameOrder === "firstLast", onClick: () => setNameOrder("firstLast") },
  ];
}

export function currentViewMenu(): MenuItem[] {
  const { view, setView } = useContactsUi.getState();
  return VIEWS.map((v) => ({ label: v.label, radio: true, checked: v.id === view, onClick: () => setView(v.id) }));
}

/** Right-click on the empty part of a contacts view. */
export function viewBackgroundMenu(): MenuItem[] {
  const g = currentGroupResource()[0];
  return [
    { label: "New Contact", icon: "contact", onClick: () => newContact(currentAccountId(), g ? { group: g } : {}) },
    { type: "separator" },
    { label: "Sort By", icon: "sort", submenu: sortMenu() },
    { label: "Display File As", submenu: nameOrderMenu() },
    { label: "Current View", submenu: currentViewMenu() },
  ];
}

/** onContextMenu for a view's scroll area: only the background (cards / rows handle their own). */
export function onViewBackgroundContextMenu(e: React.MouseEvent) {
  if ((e.target as HTMLElement).closest?.("[data-id], input")) return;
  e.preventDefault();
  showMenu(e.clientX, e.clientY, viewBackgroundMenu());
}
