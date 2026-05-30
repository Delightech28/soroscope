/**
 * Base URL of the SoroScope analyzer backend.
 *
 * Reads from NEXT_PUBLIC_API_URL (baked in at build time) and falls back to
 * localhost for local development, so no env file is needed to run locally.
 * In production, set NEXT_PUBLIC_API_URL to the deployed backend's URL.
 */
export const API_URL =
  process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:8080';

/** Build a full backend URL from a path, e.g. apiUrl('/analyze'). */
export function apiUrl(path: string): string {
  return `${API_URL}${path.startsWith('/') ? path : `/${path}`}`;
}