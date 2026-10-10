import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeRaw from "rehype-raw";
import rehypeSanitize from "rehype-sanitize";
import { asGuiError } from "./api";
import { ImageData, openScreenshotBrowser, prDescription } from "./screenshotsApi";
import { Spinner } from "./ui";

export type OpenPrImage = (image: ImageData, caption: string, trigger: HTMLElement) => void;

/** FORK(no-cloud): AIW downloads no images itself, so a PR image is a
 * placeholder that hands its link to the user's browser on request. */
function InlinePrImage({ source, caption, onError }: {
  source: string; caption: string; onError: (message: string) => void;
}) {
  const external = /^https?:\/\//i.test(source);
  return <span className="pr-inline-image muted">
    [image: {caption}]{" "}
    {external && <button type="button" className="btn btn-secondary btn-sm" onClick={(event) => {
      event.preventDefault(); event.stopPropagation();
      void openScreenshotBrowser(source).catch((err) => onError(asGuiError(err).message));
    }}>Open in browser</button>}
  </span>;
}

/** PR Markdown renders GitHub's HTML image form through sanitized nodes. Images
 * are never fetched — not by the DOM and not by AIW (see NO_CLOUD.md). */
// `workflowId` / `onOpenImage` stay in the props so callers are unchanged; with
// no image fetching (FORK(no-cloud)) they are no longer read here.
export default function PrMarkdown({ source, identity }: {
  source: string; workflowId: string; identity: string; onOpenImage: OpenPrImage;
}) {
  const [linkError, setLinkError] = useState<string | null>(null);
  const components = useMemo(() => ({
    img: ({ src, alt }: { src?: string; alt?: string }) => src
      ? <InlinePrImage key={`${identity}:${src}`} source={src}
        caption={alt || "PR image"} onError={setLinkError} />
      : <span className="muted">{alt || "Image has no supported source"}</span>,
    a: ({ href, children }: { href?: string; children?: React.ReactNode }) => {
      if (!href || !/^https?:\/\//i.test(href)) return <span>{children}</span>;
      return <a href={href} onClick={(event) => {
        event.preventDefault();
        void openScreenshotBrowser(href).catch((err) => setLinkError(asGuiError(err).message));
      }}>{children}</a>;
    },
  }), [identity]);
  return <div className="doc md pr-markdown">
    {linkError && <p role="alert">{linkError}</p>}
    <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeRaw, rehypeSanitize]}
      components={components}>{source}</ReactMarkdown>
  </div>;
}

export function PrDescription({ workflowId, identity, onOpenImage }: {
  workflowId: string; identity: string; onOpenImage: OpenPrImage;
}) {
  const [source, setSource] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let live = true;
    setSource(null);
    setError(null);
    prDescription(workflowId)
      .then((value) => { if (live) setSource(value); })
      .catch((err) => { if (live) setError(asGuiError(err).message); });
    return () => { live = false; };
  }, [workflowId, identity, retry]);
  return <details className="pr-description" open>
    <summary>PR description</summary>
    {error ? <p role="alert">{error}{" "}<button className="btn btn-secondary btn-sm"
      onClick={() => setRetry((value) => value + 1)}>Retry description</button></p>
      : source === null ? <p role="status"><Spinner /> Loading description…</p>
      : source ? <PrMarkdown source={source} workflowId={workflowId} identity={identity} onOpenImage={onOpenImage} />
      : <p className="muted">No description.</p>}
  </details>;
}
