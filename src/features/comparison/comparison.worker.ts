import { comparePixels, compareText } from "./model";

export interface WorkerInput {
  beforeText: string; afterText: string;
  beforePixels: Uint8ClampedArray; afterPixels: Uint8ClampedArray;
  width: number; height: number; tolerance: number;
}

export type WorkerOutput = {
  ok: true; text: ReturnType<typeof compareText>; pixels: ReturnType<typeof comparePixels>["pixels"]; mask: Uint8ClampedArray;
} | { ok: false; error: string };

self.onmessage = (event: MessageEvent<WorkerInput>) => {
  try {
    const input = event.data;
    const text = compareText(input.beforeText, input.afterText);
    const result = comparePixels(input.beforePixels, input.afterPixels, input.width, input.height, input.tolerance);
    const message: WorkerOutput = { ok: true, text, ...result };
    self.postMessage(message, { transfer: [result.mask.buffer] });
  } catch (error) {
    const message: WorkerOutput = { ok: false, error: error instanceof Error ? error.message : String(error) };
    self.postMessage(message);
  }
};
