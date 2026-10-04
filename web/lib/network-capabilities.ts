/** Release hold: re-enabling requires verified destination controls and review. */
export const remoteExtractionEnabled: boolean = false;
export const metadataResolutionEnabled: boolean = false;
export const remoteExtractionMessage = 'URL and webpage fetching is disabled. Upload a local file or paste text to continue.';
export function requireRemoteExtraction() {
  if (!remoteExtractionEnabled) throw new Response(remoteExtractionMessage, {status:503,headers:{'Cache-Control':'private, no-store'}});
}
