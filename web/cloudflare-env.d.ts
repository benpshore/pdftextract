declare namespace Cloudflare {
  interface Env {
    DB?: D1Database;
    BUCKET?: R2Bucket;
    /** Optional private scholarly runtime; configured only by an explicit local preview. */
    SCHOLARLY?: Fetcher;
  }
}
