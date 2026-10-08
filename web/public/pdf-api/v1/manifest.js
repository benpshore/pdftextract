// Artifacts are staged explicitly by scripts/stage-docling-assets.mjs, never per job.
export const VERSION = '1.104.2';
export const COMMIT = '29de9d1e842e6ebb35890c3f171ac0c38f273eed';
export const ROOT = '/vendor/docling/1.104.2/';
export const MODELS = {
  layout: {file:'layout_heron_int8.onnx', bytes:68695321, sha256:'387be1f67bd58ec7fc575c53966492890d91af0810866bb383af45a18cc66161'},
  recognition: {file:'ocr_rec_en.onnx', bytes:8967018, sha256:'ef7abd8bd3629ae57ea2c28b425c1bd258a871b93fd2fe7c433946ade9b5d9ea'},
  detector: {file:'ocr_det.onnx', bytes:9929594, sha256:'090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f'},
  dictionary: {file:'en_dict.txt', bytes:190, sha256:'5662df9d2d03f0e8ca0d3b0649d6acbab904b6a14b3d3521463c71c37c668ce3'},
};
export const LIMITS = Object.freeze({maxBytes:25*1024*1024,maxPages:20,maxDocumentPages:200,maxPixels:6000000,maxTextCharacters:1000000,maxTextCells:100000,maxOutputBytes:8*1024*1024,timeoutMs:120000});
export const RUNTIME_POLICY = Object.freeze({idleMs:30000,maxRetainedRustWasmBytes:256*1024*1024});
