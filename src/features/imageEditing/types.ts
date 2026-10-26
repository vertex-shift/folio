export type ImagePlacement = [number, number, number, number];

export interface ImageOccurrence {
  path: number[];
  name: string;
  pixelWidth: number | null;
  pixelHeight: number | null;
  placement: ImagePlacement | null;
  editable: boolean;
  reason: string | null;
}

export interface ImagePage {
  images: ImageOccurrence[];
  warnings: string[];
}

export interface ImageEditDraft {
  documentId: number;
  revision: number;
  page: number;
  source: ImageOccurrence;
  x: string;
  y: string;
  width: string;
  height: string;
  remove: boolean;
  replacement: { assetId: number; width: number; height: number; name: string; previewUrl: string } | null;
}

export type ImageChange =
  | { action: "remove"; page: number; path: number[] }
  | { action: "place"; page: number; path: number[]; placement: ImagePlacement }
  | { action: "replace"; page: number; path: number[]; assetId: number; placement: ImagePlacement | null };

export interface ImageInspection {
  loading: boolean;
  error: string | null;
  result: ImagePage | null;
}
