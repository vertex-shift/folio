import { useEffect, useRef, useState } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { errorMessage } from "./geometry";
import { excerpt, findMatches, mapText } from "./search";
import { textRevisionKey } from "./existingText";
import type { DocumentInfo, SearchResult } from "./types";

interface State {
  results: SearchResult[];
  loading: boolean;
  progress: number;
  error: string | null;
  limited: boolean;
  noText: boolean;
}

const empty: State = { results: [], loading: false, progress: 0, error: null, limited: false, noText: false };

export function useSearch(
  pdf: PDFDocumentProxy | null,
  info: DocumentInfo | null,
  query: string,
  matchCase: boolean,
  enabled: boolean,
) {
  const [state, setState] = useState<State>(empty);
  const cache = useRef(new Map<string, string>());
  const cacheDocument = useRef<string | null>(null);

  useEffect(() => {
    const revision = info ? textRevisionKey(info) : null;
    if (cacheDocument.current !== revision) {
      cache.current.clear();
      cacheDocument.current = revision;
    }
    if (!pdf || !info || !query.trim() || !enabled) {
      setState(empty);
      return;
    }
    let cancelled = false;
    setState({ ...empty, loading: true });
    const timeout = window.setTimeout(() => {
      async function scan() {
        if (!pdf || !info) return;
        const results: SearchResult[] = [];
        let textLength = 0;
        let page = 0;
        try {
          for (; page < info.pageCount; page++) {
            if (cancelled) return;
            const key = info.pages[page].key;
            let text = cache.current.get(key);
            if (text === undefined) {
              const pdfPage = await pdf.getPage(page + 1);
              const content = await pdfPage.getTextContent();
              if (cancelled) return;
              text = mapText(content.items.filter((item) => "str" in item)).text;
              cache.current.set(key, text);
            }
            textLength += text.length;
            const matches = findMatches(text, query, matchCase);
            matches.forEach((match, index) => {
              if (results.length <= 500) results.push({ page, index, ...excerpt(text, match) });
            });
            if (results.length > 500) {
              setState({ results: results.slice(0, 500), loading: false, progress: page + 1, error: null, limited: true, noText: false });
              return;
            }
            if (page % 4 === 0) {
              setState((state) => ({ ...state, progress: page + 1 }));
              await new Promise((resolve) => window.setTimeout(resolve, 0));
            }
          }
          if (!cancelled) setState({ results, loading: false, progress: page, error: null, limited: false, noText: textLength === 0 });
        } catch (error) {
          if (!cancelled) setState({
            ...empty, results,
            error: `Search could not read page ${page + 1}: ${errorMessage(error)}`,
          });
        }
      }
      void scan();
    }, 180);
    return () => { cancelled = true; window.clearTimeout(timeout); };
  }, [pdf, info, query, matchCase, enabled]);

  return state;
}
