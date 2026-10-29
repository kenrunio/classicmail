// Getting new-mail batches to the Desktop Alert window without losing any (alerts.ts wires it to
// Tauri). One step at a time: two batches at once open the window once. A window that is opening
// can't hear events yet, so batches wait until it says it is ready; a window that is closing takes
// none, and what came meanwhile opens a new one once it is gone.

/** What the queue needs from the window (fakes in the tests). */
export interface AlertWindowOps<T> {
  exists: () => Promise<boolean>;
  /** open the window showing `batch`; resolves once it exists, rejects if it can't be made */
  create: (batch: T) => Promise<void>;
  send: (batch: T) => Promise<void>;
  log: (message: string) => void;
}

const GONE_POLL_MS = 100;
const GONE_WAIT_MS = 3000;

export class AlertQueue<T> {
  private step: Promise<void> = Promise.resolve();
  /** the window is listening */
  private ready = false;
  /** the window is closing (or opening): batches wait here */
  private waiting: T[] = [];
  private closing = false;

  constructor(
    private ops: AlertWindowOps<T>,
    private sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms)),
  ) {}

  /** Runs `f` after everything queued before it; a failure is logged and doesn't stop the queue. */
  private then(f: () => Promise<void>): Promise<void> {
    this.step = this.step.then(f).catch((e) => this.ops.log(`desktop alert: ${String(e)}`));
    return this.step;
  }

  show(batch: T): Promise<void> {
    return this.then(async () => {
      const exists = await this.ops.exists();
      if (this.ready && exists && !this.closing) return this.ops.send(batch);
      this.waiting.push(batch);
      if (!exists && !this.closing) await this.openWaiting();
    });
  }

  /** The window listens now: what came while it opened goes to it. */
  onReady(): Promise<void> {
    return this.then(async () => {
      this.ready = true;
      this.closing = false;
      for (const b of this.waiting.splice(0)) await this.ops.send(b);
    });
  }

  /** The window is about to close: nothing more goes to it; anything waiting opens a new one. */
  onClosing(): Promise<void> {
    this.ready = false;
    this.closing = true;
    return this.then(async () => {
      for (let t = 0; t < GONE_WAIT_MS && (await this.ops.exists()); t += GONE_POLL_MS) await this.sleep(GONE_POLL_MS);
      this.closing = false;
      await this.openWaiting();
    });
  }

  /** A new window with the first waiting batch; the others go to it once it is ready. */
  private async openWaiting() {
    this.ready = false;
    const first = this.waiting.shift();
    if (first === undefined) return;
    try {
      await this.ops.create(first);
    } catch (e) {
      this.ops.log(`desktop alert window couldn't be opened: ${String(e)}`);
    }
  }
}
