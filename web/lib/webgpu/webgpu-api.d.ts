/** Minimal ambient WebGPU declarations for the subset this directory uses.
 * TypeScript's bundled DOM library declares only GPUError; these follow the
 * W3C WebGPU IDL names so a future lib.dom/@webgpu/types can replace them. */
interface GPUAdapterInfo { readonly vendor: string; readonly architecture: string; readonly device: string; readonly description: string; readonly isFallbackAdapter?: boolean }
interface GPUSupportedLimits {
  readonly maxBufferSize: number; readonly maxStorageBufferBindingSize: number; readonly maxUniformBufferBindingSize: number;
  readonly maxComputeWorkgroupsPerDimension: number; readonly maxComputeInvocationsPerWorkgroup: number; readonly maxComputeWorkgroupSizeX: number;
  readonly maxComputeWorkgroupStorageSize: number; readonly maxStorageBuffersPerShaderStage: number;
}
interface GPUSupportedFeatures extends ReadonlySet<string> { readonly size: number }
interface GPUDeviceLostInfo { readonly reason: 'destroyed' | 'unknown'; readonly message: string }
interface GPUBuffer { readonly size: number; mapAsync(mode: number, offset?: number, size?: number): Promise<void>; getMappedRange(offset?: number, size?: number): ArrayBuffer; unmap(): void; destroy(): void }
interface GPUShaderModule { getCompilationInfo?(): Promise<{ messages: ReadonlyArray<{ type: string; message: string; lineNum: number }> }> }
interface GPUBindGroupLayout { readonly label?: string }
interface GPUBindGroup { readonly label?: string }
interface GPUComputePipeline { getBindGroupLayout(index: number): GPUBindGroupLayout }
interface GPUComputePassEncoder { setPipeline(pipeline: GPUComputePipeline): void; setBindGroup(index: number, group: GPUBindGroup): void; dispatchWorkgroups(x: number, y?: number, z?: number): void; end(): void }
interface GPUCommandBuffer { readonly label?: string }
interface GPUCommandEncoder { beginComputePass(): GPUComputePassEncoder; copyBufferToBuffer(source: GPUBuffer, sourceOffset: number, destination: GPUBuffer, destinationOffset: number, size: number): void; clearBuffer(buffer: GPUBuffer, offset?: number, size?: number): void; finish(): GPUCommandBuffer }
interface GPUQueue { submit(buffers: GPUCommandBuffer[]): void; writeBuffer(buffer: GPUBuffer, offset: number, data: BufferSource, dataOffset?: number, size?: number): void; onSubmittedWorkDone(): Promise<void> }
interface GPUBufferDescriptor { label?: string; size: number; usage: number; mappedAtCreation?: boolean }
interface GPUBindGroupEntry { binding: number; resource: { buffer: GPUBuffer; offset?: number; size?: number } }
interface GPUDevice extends EventTarget {
  readonly features: GPUSupportedFeatures; readonly limits: GPUSupportedLimits; readonly queue: GPUQueue; readonly lost: Promise<GPUDeviceLostInfo>;
  createBuffer(descriptor: GPUBufferDescriptor): GPUBuffer;
  createShaderModule(descriptor: { label?: string; code: string }): GPUShaderModule;
  createComputePipelineAsync(descriptor: { label?: string; layout: 'auto'; compute: { module: GPUShaderModule; entryPoint: string } }): Promise<GPUComputePipeline>;
  createBindGroup(descriptor: { label?: string; layout: GPUBindGroupLayout; entries: GPUBindGroupEntry[] }): GPUBindGroup;
  createCommandEncoder(descriptor?: { label?: string }): GPUCommandEncoder;
  pushErrorScope(filter: 'validation' | 'out-of-memory' | 'internal'): void;
  popErrorScope(): Promise<GPUError | null>;
  destroy(): void;
}
interface GPUAdapter {
  readonly features: GPUSupportedFeatures; readonly limits: GPUSupportedLimits; readonly info?: GPUAdapterInfo; readonly isFallbackAdapter?: boolean;
  requestAdapterInfo?(): Promise<GPUAdapterInfo>;
  requestDevice(descriptor?: { label?: string; requiredFeatures?: string[]; requiredLimits?: Record<string, number> }): Promise<GPUDevice>;
}
interface GPU { requestAdapter(options?: { powerPreference?: 'low-power' | 'high-performance'; forceFallbackAdapter?: boolean }): Promise<GPUAdapter | null> }
interface Navigator { readonly gpu?: GPU }
interface WorkerNavigator { readonly gpu?: GPU }
declare const GPUBufferUsage: { readonly MAP_READ: number; readonly MAP_WRITE: number; readonly COPY_SRC: number; readonly COPY_DST: number; readonly UNIFORM: number; readonly STORAGE: number };
declare const GPUMapMode: { readonly READ: number; readonly WRITE: number };
