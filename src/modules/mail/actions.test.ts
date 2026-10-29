import { beforeEach, describe, expect, it, vi } from "vitest";

const deleteInfo = vi.fn();
const messageBox = vi.fn();
const undoable = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: vi.fn() }));
vi.mock("../../api/windows", () => ({ openWindow: vi.fn() }));
vi.mock("../../api/mail", () => ({ mailApi: { deleteInfo }, folderOf: vi.fn(), inFolder: vi.fn() }));
vi.mock("../../ui", () => ({ messageBox, alertBox: vi.fn(), confirmBox: vi.fn(), errorBox: vi.fn() }));
vi.mock("../../api/undo", () => ({ undoable, items: (n: number) => `${n} item${n === 1 ? "" : "s"}` }));
vi.mock("./dialogs", () => ({}));

const { deleteItems, permanentDeleteText } = await import("./actions");

describe("deleteItems", () => {
  beforeEach(() => {
    deleteInfo.mockReset();
    messageBox.mockReset();
    undoable.mockReset().mockResolvedValue(undefined);
  });

  it("deletes without asking when nothing is erased for good", async () => {
    deleteInfo.mockResolvedValue({ permanent: 0 });
    expect(await deleteItems([1, 2])).toBe(true);
    expect(messageBox).not.toHaveBeenCalled();
    expect(undoable).toHaveBeenCalledWith("Delete 2 items", "mail.delete", { ids: [1, 2] });
  });

  it("asks first, and No does nothing", async () => {
    deleteInfo.mockResolvedValue({ permanent: 1 });
    messageBox.mockResolvedValue("No");
    expect(await deleteItems([7])).toBe(false);
    expect(messageBox.mock.calls[0][0].text).toBe("The selected item will be permanently deleted.");
    expect(undoable).not.toHaveBeenCalled();
  });

  it("Yes deletes; a failed delete reports false", async () => {
    deleteInfo.mockResolvedValue({ permanent: 2 });
    messageBox.mockResolvedValue("Yes");
    expect(await deleteItems([1, 2])).toBe(true);
    undoable.mockRejectedValue(new Error("offline"));
    expect(await deleteItems([1, 2])).toBe(false);
  });
});

describe("permanentDeleteText", () => {
  it("says how many of the selection go for good", () => {
    expect(permanentDeleteText(3, 3)).toBe("The 3 selected items will be permanently deleted.");
    expect(permanentDeleteText(3, 5)).toBe("3 of the 5 selected items will be permanently deleted. The others will be moved to the Deleted Items folder.");
  });
});
