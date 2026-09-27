/**
 * LocalStorage helpers with AES-GCM encryption for sensitive Soroban RPC
 * endpoint configuration (custom RPC URLs and secret keys).
 *
 * Encrypted values are stored as JSON envelopes of the form:
 *   { __enc: true, iv: <base64>, data: <base64> }
 * so that plaintext values written by older versions remain readable.
 */

const ENCRYPTION_KEY_STORAGE = 'soroban_rpc_enc_key';
const RPC_ENDPOINTS_STORAGE = 'soroban_rpc_endpoints';
const RPC_SECRET_STORAGE = 'soroban_rpc_secret';

interface EncryptedEnvelope {
  __enc: true;
  iv: string;
  data: string;
}

function isEncryptedEnvelope(value: unknown): value is EncryptedEnvelope {
  return (
    typeof value === 'object' &&
    value !== null &&
    (value as { __enc?: unknown }).__enc === true &&
    typeof (value as { iv?: unknown }).iv === 'string' &&
    typeof (value as { data?: unknown }).data === 'string'
  );
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

function base64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

async function getEncryptionKey(): Promise<CryptoKey> {
  const stored = localStorage.getItem(ENCRYPTION_KEY_STORAGE);
  if (stored) {
    const raw = base64ToBytes(stored);
    return crypto.subtle.importKey('raw', raw, { name: 'AES-GCM' }, false, [
      'encrypt',
      'decrypt',
    ]);
  }

  const key = await crypto.subtle.generateKey(
    { name: 'AES-GCM', length: 256 },
    true,
    ['encrypt', 'decrypt'],
  );
  const exported = new Uint8Array(await crypto.subtle.exportKey('raw', key));
  localStorage.setItem(ENCRYPTION_KEY_STORAGE, bytesToBase64(exported));
  return key;
}

async function encryptValue(plaintext: string): Promise<EncryptedEnvelope> {
  const key = await getEncryptionKey();
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const encoded = new TextEncoder().encode(plaintext);
  const cipher = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, encoded);
  return {
    __enc: true,
    iv: bytesToBase64(iv),
    data: bytesToBase64(new Uint8Array(cipher)),
  };
}

async function decryptValue(envelope: EncryptedEnvelope): Promise<string> {
  const key = await getEncryptionKey();
  const iv = base64ToBytes(envelope.iv);
  const data = base64ToBytes(envelope.data);
  const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, data);
  return new TextDecoder().decode(plain);
}

async function readEncrypted<T>(storageKey: string): Promise<T | null> {
  const raw = localStorage.getItem(storageKey);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (isEncryptedEnvelope(parsed)) {
      return JSON.parse(await decryptValue(parsed)) as T;
    }
    return parsed as T;
  } catch {
    return null;
  }
}

async function writeEncrypted(storageKey: string, value: unknown): Promise<void> {
  const envelope = await encryptValue(JSON.stringify(value));
  localStorage.setItem(storageKey, JSON.stringify(envelope));
}

/**
 * Persist the list of custom Soroban RPC endpoints, encrypting the payload
 * with AES-GCM before it touches LocalStorage.
 */
export async function saveRpcEndpoints(endpoints: string[]): Promise<void> {
  await writeEncrypted(RPC_ENDPOINTS_STORAGE, endpoints);
}

/**
 * Read and decrypt the stored custom Soroban RPC endpoints.
 */
export async function getRpcEndpoints(): Promise<string[]> {
  const endpoints = await readEncrypted<string[]>(RPC_ENDPOINTS_STORAGE);
  return Array.isArray(endpoints) ? endpoints : [];
}

/**
 * Persist a Soroban secret key, encrypting it with AES-GCM before storage.
 */
export async function saveRpcSecret(secret: string): Promise<void> {
  await writeEncrypted(RPC_SECRET_STORAGE, secret);
}

/**
 * Read and decrypt the stored Soroban secret key.
 */
export async function getRpcSecret(): Promise<string | null> {
  return readEncrypted<string>(RPC_SECRET_STORAGE);
}

/**
 * Remove every stored Soroban RPC endpoint and secret from LocalStorage.
 * Backs the "Clear All Stored Endpoints" settings action.
 */
export function clearAllStoredEndpoints(): void {
  localStorage.removeItem(RPC_ENDPOINTS_STORAGE);
  localStorage.removeItem(RPC_SECRET_STORAGE);
}
