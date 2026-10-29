// Writing the Rules and Alerts working copy to the database (Apply / OK), in steps that a failure
// part way can pick up from: what already saved is never created or deleted a second time.
import type { Rule } from "../../api/rules";

export interface ApplyApi {
  save: (rule: Rule) => Promise<{ id: number }>;
  delete: (ids: number[]) => Promise<unknown>;
  reorder: (ids: number[]) => Promise<unknown>;
}

/** What has been written so far (filled in as each call succeeds). */
export interface ApplyProgress {
  /** working-copy id → saved id (new rules have negative temp ids until saved) */
  ids: Map<number, number>;
  /** the deleted rules are gone from the database */
  deletedDone: boolean;
}

const comparable = (r: Rule) => JSON.stringify({ ...r, position: 0, createdAt: 0, updatedAt: 0 });

/** Delete `dels`, create or update the rules of `list` that differ from `saved`, then store the order. */
export async function applyRules(list: Rule[], dels: number[], saved: Rule[], api: ApplyApi, progress: ApplyProgress): Promise<void> {
  const gone = dels.filter((id) => id > 0);
  if (gone.length) await api.delete(gone);
  progress.deletedDone = true;
  const order: number[] = [];
  for (const r of list) {
    if (r.id < 0) {
      const { id } = await api.save({ ...r, id: 0 });
      progress.ids.set(r.id, id);
      order.push(id);
    } else {
      const orig = saved.find((o) => o.id === r.id);
      if (!orig || comparable(orig) !== comparable(r)) await api.save(r);
      progress.ids.set(r.id, r.id);
      order.push(r.id);
    }
  }
  if (order.length) await api.reorder(order);
}

/** After a failed Apply: the rules created so far under their saved ids, so the next Apply updates
 *  them instead of creating them again. */
export function withSavedIds(list: Rule[], ids: Map<number, number>): Rule[] {
  return list.map((r) => (r.id < 0 && ids.has(r.id) ? { ...r, id: ids.get(r.id)! } : r));
}
