import type { LinkEvidence } from '../types';

export type ArticleAuthor = { name: string; url?: string; affiliation?: string };
export type ArticleImage = { id: string; url: string; alt: string; caption: string; width?: string; height?: string; source: 'article' };
export type ArticleHeading = { id: string; level: number; title: string };
export type ArticleFeed = { type: string | null; url: string };

/** Identifiers and bibliographic fields found in citation_*, Dublin Core, PRISM and JSON-LD metadata. */
export type ScholarlyMetadata = {
  type?: string;
  doi?: string;
  pmid?: string;
  pmcid?: string;
  arxiv?: string;
  journal?: string;
  volume?: string;
  issue?: string;
  firstPage?: string;
  lastPage?: string;
  publisher?: string;
  issn?: string;
  isbn?: string;
  pdfUrl?: string;
  abstract?: string;
  keywords?: string[];
};

export type PaywallInfo = { detected: boolean; evidence: string[] };
export type AmpInfo = { isAmp: boolean; ampUrl: string | null };

export type ArticleLimits = {
  /** Input characters kept before the HTML is parsed; longer sources are cut and repaired. */
  maxInputChars: number;
  maxImages: number;
  maxLinks: number;
  maxTables: number;
  /** Upper bound of DOM elements the article scorer inspects before it falls back to the cleaned boundary. */
  maxElements: number;
  /** Pages followed through rel="next" links, including the first page. */
  maxPages: number;
};

export type ExtractOptions = {
  /** The source is a trusted fragment (feed content); skip page-level boilerplate removal and scoring. */
  fragment?: boolean;
  /** Title to use when the page provides none. */
  fallbackTitle?: string;
  limits?: Partial<ArticleLimits>;
};

export type SelectionMode = 'readability' | 'structured main content' | 'scored block' | 'main boundary' | 'body' | 'fragment';

export type Article = {
  title: string;
  authors: ArticleAuthor[];
  byline: string | null;
  published: string | null;
  modified: string | null;
  sourceUrl: string;
  canonicalUrl: string | null;
  siteName: string | null;
  language: string | null;
  description: string | null;
  html: string;
  markdown: string;
  text: string;
  headings: ArticleHeading[];
  images: ArticleImage[];
  links: LinkEvidence[];
  tables: string[][][];
  feeds: ArticleFeed[];
  scholarly: ScholarlyMetadata | null;
  structuredData: Record<string, unknown>[];
  meta: Record<string, string>;
  amp: AmpInfo;
  paywall: PaywallInfo;
  nextPageUrl: string | null;
  /** Source URLs of every page merged into this article, in reading order. */
  pages: string[];
  selection: SelectionMode;
  truncated: boolean;
  repairs: string[];
  warnings: string[];
};
