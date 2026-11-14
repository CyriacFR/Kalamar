// --probe-blockviews: does the GPU keep its writes inside a BC image made for block views? No game files.
// vk_texture.c create_tex makes a block-compressed image BLOCK_TEXEL_VIEW_COMPATIBLE with COLOR_ATTACHMENT usage so
// the game's draws into it go through an uncompressed view of its blocks. On AMD's RDNA 1/2 Windows driver (RX 6600,
// 26.10.1) such an image takes GPU writes past the size vkGetImageMemoryRequirements reports: the textures behind it
// in the memory block are overwritten, and the device is lost when the image ends the block.
// Each case binds the image near the start of a device memory of its own and a guard buffer right behind it, fills the
// guard with a pattern, runs one kind of write on the image (nothing but the first layout transition, a buffer copy
// into every level, an image copy of blocks from an uncompressed image into every level as vk_exec.c bc_copy_back does
// for a stand-in, a cleared render pass through a block view of every level) and reads the guard back: any changed
// byte is a write past the image. The memory has slack behind the guard, so an overrun stays in allocated memory.
// The copies also read level 0 back and compare it with what was copied in. Each case runs without and with the block
// view flags. Exit 0 when no case writes past its image or loses its blocks, 3 when one does.
// "stand-in pass" is what vk_exec.c does for a 2D BC texture without block views (bc_target_flat, flat_standin,
// bc_refresh, bc_copy_flat), on an image without the block view flags: blocks copied into every level, then per level
// a copy of the level into the top-left corner of one uncompressed image of level 0's blocks rounded up to 16 (the
// stand-in, a plain colour attachment shared by every level), a pass over the level's blocks that loads it and clears
// a quarter of layer 0's blocks at level 0, and the copy of the level's blocks back. Level 0 must read back as the
// source with that quarter cleared.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "platform/platform.h"
#include "host.h"
#include "vk.h"

enum { GUARD = 1 << 20, SLACK = 8 << 20, PATTERN = 0xA5 };
enum { OP_INIT, OP_COPY, OP_IMAGE, OP_STANDIN, OP_CLEAR, OP_N };
static const char *const op_name[OP_N] = {"layout transition only", "buffer copy into every level",
                                          "image copy of blocks into every level",
                                          "stand-in pass of every level (copy in, clear, copy back)",
                                          "cleared pass through a block view of every level"};
enum { SI_CLEAR = 0x5A5A };   // the stand-in pass's clear value in every channel (fits 16-bit channels)

typedef struct { VkFormat fmt; const char *name; uint32_t bpb, w, h, levels, layers; } Case;
// The first three are images the game makes at the shooting range (format, size, levels, layers as logged there).
static const Case cases[] = {
    {VK_FORMAT_BC1_RGBA_SRGB_BLOCK, "BC1", 8, 64, 64, 7, 1},     {VK_FORMAT_BC3_UNORM_BLOCK, "BC3", 16, 256, 256, 9, 6},
    {VK_FORMAT_BC4_UNORM_BLOCK, "BC4", 8, 128, 128, 1, 1},       {VK_FORMAT_BC3_UNORM_BLOCK, "BC3", 16, 64, 64, 7, 1},
    {VK_FORMAT_BC5_UNORM_BLOCK, "BC5", 16, 192, 48, 1, 1},       {VK_FORMAT_BC7_UNORM_BLOCK, "BC7", 16, 512, 512, 10, 1},
    {VK_FORMAT_BC1_RGBA_UNORM_BLOCK, "BC1", 8, 1024, 1024, 11, 1}, {VK_FORMAT_BC4_UNORM_BLOCK, "BC4", 8, 32, 32, 6, 1},
    // sizes that are no multiple of the block, as the textures the game encodes in the plaza
    {VK_FORMAT_BC3_SRGB_BLOCK, "BC3", 16, 1230, 1790, 10, 1},     {VK_FORMAT_BC3_SRGB_BLOCK, "BC3", 16, 90, 235, 7, 1},
};

static VkCommandPool g_pool;

static void all_barrier(VkCommandBuffer cb) {
    VkMemoryBarrier2 mb = {.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2, .srcStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                           .srcAccessMask = VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT,
                           .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                           .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT};
    VkDependencyInfo di = {.sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO, .memoryBarrierCount = 1, .pMemoryBarriers = &mb};
    vkCmdPipelineBarrier2(cb, &di);
}

// One case: -1 the device cannot make the image or the run failed (said on stdout), -2 device lost, else the number
// of guard bytes the GPU changed.
static long run_case(VkCtx *c, const Case *k, int bv, int op) {
    long result = -1;
    VkImageCreateInfo ci = {.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO, .flags = VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT,
                            .imageType = VK_IMAGE_TYPE_2D, .format = k->fmt, .extent = {k->w, k->h, 1},
                            .mipLevels = k->levels, .arrayLayers = k->layers, .samples = VK_SAMPLE_COUNT_1_BIT,
                            .tiling = VK_IMAGE_TILING_OPTIMAL, .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED,
                            .usage = VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT};
    if (bv) {
        ci.flags |= VK_IMAGE_CREATE_BLOCK_TEXEL_VIEW_COMPATIBLE_BIT | VK_IMAGE_CREATE_EXTENDED_USAGE_BIT;
        ci.usage |= VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
    }
    VkPhysicalDeviceImageFormatInfo2 fi = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2, .format = ci.format,
                                           .type = ci.imageType, .tiling = ci.tiling, .usage = ci.usage, .flags = ci.flags};
    VkImageFormatProperties2 fp = {.sType = VK_STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2};
    if (vkGetPhysicalDeviceImageFormatProperties2(c->phys, &fi, &fp) != VK_SUCCESS) {
        printf("probe-blockviews: %s %ux%u: the device refuses the image%s\n", k->name, k->w, k->h, bv ? " with block views" : "");
        return -1;
    }
    VkFormat vfmt = k->bpb == 8 ? VK_FORMAT_R16G16B16A16_UINT : VK_FORMAT_R32G32B32A32_UINT;
    VkImage img = VK_NULL_HANDLE;
    VkBuffer guard = VK_NULL_HANDLE, src = VK_NULL_HANDLE, back = VK_NULL_HANDLE, lvl = VK_NULL_HANDLE;
    VmaAllocation src_a = NULL, back_a = NULL, lvl_a = NULL, blk_a = NULL;
    VkImage blk = VK_NULL_HANDLE;   // OP_IMAGE: level 0's blocks as texels of the uncompressed format
    VkDeviceMemory mem = VK_NULL_HANDLE;
    VkCommandBuffer cb = VK_NULL_HANDLE;
    VkFence fence = VK_NULL_HANDLE;
    VkImageView views[16] = {0};
    VkImage si_img[16] = {0};          // OP_STANDIN: each level's stand-in
    VmaAllocation si_a[16] = {0};
    uint32_t bw0 = (k->w + 3) / 4, bh0 = (k->h + 3) / 4;   // level 0 in blocks; the stand-in pass clears a quarter
    uint32_t cw = bw0 / 2 ? bw0 / 2 : 1, chh = bh0 / 2 ? bh0 / 2 : 1;
    if (vkCreateImage(c->device, &ci, NULL, &img) != VK_SUCCESS) goto out;
    VkMemoryRequirements rq, rb;
    vkGetImageMemoryRequirements(c->device, img, &rq);
    VkBufferCreateInfo gi = {.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = GUARD,
                             .usage = VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT};
    if (vkCreateBuffer(c->device, &gi, NULL, &guard) != VK_SUCCESS) goto out;
    vkGetBufferMemoryRequirements(c->device, guard, &rb);
    uint32_t type = UINT32_MAX;
    for (uint32_t i = 0; i < c->mem_props.memoryTypeCount && type == UINT32_MAX; i++)
        if ((rq.memoryTypeBits & rb.memoryTypeBits & 1u << i) &&
            (c->mem_props.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT))
            type = i;
    if (type == UINT32_MAX) goto out;
    VkDeviceSize ioff = rq.alignment * 37;   // as in a block shared with other images: no page boundary
    VkDeviceSize goff = (ioff + rq.size + rb.alignment - 1) / rb.alignment * rb.alignment;
    VkMemoryAllocateInfo mi = {.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, .allocationSize = goff + GUARD + SLACK,
                               .memoryTypeIndex = type};
    if (vkAllocateMemory(c->device, &mi, NULL, &mem) != VK_SUCCESS) goto out;
    if (vkBindImageMemory(c->device, img, mem, ioff) != VK_SUCCESS || vkBindBufferMemory(c->device, guard, mem, goff) != VK_SUCCESS)
        goto out;
    // the copy source: level 0 of every layer, the same bytes for every level
    VkDeviceSize src_size = (VkDeviceSize)((k->w + 3) / 4) * ((k->h + 3) / 4) * k->bpb * k->layers;
    VmaAllocationCreateInfo hi = {.usage = VMA_MEMORY_USAGE_AUTO,
                                  .flags = VMA_ALLOCATION_CREATE_HOST_ACCESS_RANDOM_BIT | VMA_ALLOCATION_CREATE_MAPPED_BIT};
    VmaAllocationInfo src_i, back_i, lvl_i;
    VkBufferCreateInfo si = {.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = src_size, .usage = VK_BUFFER_USAGE_TRANSFER_SRC_BIT};
    VkBufferCreateInfo bi = {.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = GUARD, .usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT};
    if (vmaCreateBuffer(c->vma, &si, &hi, &src, &src_a, &src_i) != VK_SUCCESS ||
        vmaCreateBuffer(c->vma, &bi, &hi, &back, &back_a, &back_i) != VK_SUCCESS)
        goto out;
    VkBufferCreateInfo li = {.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = src_size, .usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT};
    if (vmaCreateBuffer(c->vma, &li, &hi, &lvl, &lvl_a, &lvl_i) != VK_SUCCESS) goto out;
    for (VkDeviceSize i = 0; i < src_size; i++) ((uint8_t *)src_i.pMappedData)[i] = (uint8_t)(i * 7 + (i >> 8) * 13 + 3);
    memset(back_i.pMappedData, 0, GUARD);
    memset(lvl_i.pMappedData, 0, (size_t)src_size);
    if (op == OP_IMAGE) {
        VkImageCreateInfo ui = {.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO, .imageType = VK_IMAGE_TYPE_2D, .format = vfmt,
                                .extent = {(k->w + 3) / 4, (k->h + 3) / 4, 1}, .mipLevels = 1, .arrayLayers = k->layers,
                                .samples = VK_SAMPLE_COUNT_1_BIT, .tiling = VK_IMAGE_TILING_OPTIMAL,
                                .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                                         VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                                .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED};
        VmaAllocationCreateInfo di2 = {.usage = VMA_MEMORY_USAGE_AUTO_PREFER_DEVICE};
        if (vmaCreateImage(c->vma, &ui, &di2, &blk, &blk_a, NULL) != VK_SUCCESS) goto out;
    }
    VkCommandBufferAllocateInfo ca = {.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO, .commandPool = g_pool,
                                      .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY, .commandBufferCount = 1};
    if (vkAllocateCommandBuffers(c->device, &ca, &cb) != VK_SUCCESS) goto out;
    VkCommandBufferBeginInfo bg = {.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                   .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT};
    vkBeginCommandBuffer(cb, &bg);
    vkCmdFillBuffer(cb, guard, 0, GUARD, PATTERN * 0x01010101u);
    all_barrier(cb);
    VkImageMemoryBarrier2 ib = {.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2, .srcStageMask = VK_PIPELINE_STAGE_2_NONE,
                                .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                                .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT,
                                .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED, .newLayout = VK_IMAGE_LAYOUT_GENERAL,   // as init_image
                                .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED, .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                .image = img, .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, k->levels, 0, k->layers}};
    VkDependencyInfo di = {.sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO, .imageMemoryBarrierCount = 1, .pImageMemoryBarriers = &ib};
    vkCmdPipelineBarrier2(cb, &di);
    if (op == OP_IMAGE) {   // the uncompressed image takes the same bytes the buffer copy puts into level 0
        ib.image = blk, ib.subresourceRange.levelCount = 1;
        vkCmdPipelineBarrier2(cb, &di);
        VkBufferImageCopy r = {.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, k->layers},
                               .imageExtent = {(k->w + 3) / 4, (k->h + 3) / 4, 1}};
        vkCmdCopyBufferToImage(cb, src, blk, VK_IMAGE_LAYOUT_GENERAL, 1, &r);
        all_barrier(cb);
    }
    if (op == OP_STANDIN) {   // the texture's blocks first, as an upload would leave them
        for (uint32_t l = 0; l < k->levels; l++) {
            uint32_t lw = k->w >> l ? k->w >> l : 1, lh = k->h >> l ? k->h >> l : 1;
            VkBufferImageCopy r = {.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, l, 0, k->layers}, .imageExtent = {lw, lh, 1}};
            vkCmdCopyBufferToImage(cb, src, img, VK_IMAGE_LAYOUT_GENERAL, 1, &r);
        }
        all_barrier(cb);
    }
    for (uint32_t l = 0; l < k->levels && op != OP_INIT; l++) {
        uint32_t lw = k->w >> l ? k->w >> l : 1, lh = k->h >> l ? k->h >> l : 1;
        if (op == OP_STANDIN) {   // as vk_exec.c: flat_standin, bc_refresh, a pass that loads, bc_copy_flat
            // smallest level first: level 0 goes last through a stand-in the other levels left their blocks in
            uint32_t sl = k->levels - 1 - l;
            lw = k->w >> sl ? k->w >> sl : 1, lh = k->h >> sl ? k->h >> sl : 1;
            uint32_t sw = (lw + 3) / 4, sh = (lh + 3) / 4;
            if (l == 0) {   // one stand-in for every level, rounded up to 16 blocks as vk_exec.c flat_standin makes it
                VkImageCreateInfo ui = {.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO, .imageType = VK_IMAGE_TYPE_2D,
                                        .format = vfmt, .extent = {(bw0 + 15) & ~15u, (bh0 + 15) & ~15u, 1}, .mipLevels = 1,
                                        .arrayLayers = k->layers, .samples = VK_SAMPLE_COUNT_1_BIT,
                                        .tiling = VK_IMAGE_TILING_OPTIMAL,
                                        .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                                                 VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                                        .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED};
                VmaAllocationCreateInfo da = {.usage = VMA_MEMORY_USAGE_AUTO_PREFER_DEVICE};
                if (vmaCreateImage(c->vma, &ui, &da, &si_img[0], &si_a[0], NULL) != VK_SUCCESS) {
                    printf("probe-blockviews: %s %ux%u: no stand-in\n", k->name, k->w, k->h);
                    vkEndCommandBuffer(cb);
                    goto out;
                }
                VkImageMemoryBarrier2 sb = ib;
                sb.image = si_img[0], sb.subresourceRange = (VkImageSubresourceRange){VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, k->layers};
                VkDependencyInfo sd = {.sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO, .imageMemoryBarrierCount = 1,
                                       .pImageMemoryBarriers = &sb};
                vkCmdPipelineBarrier2(cb, &sd);
                VkImageViewCreateInfo vi = {.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO, .image = si_img[0],
                                            .viewType = VK_IMAGE_VIEW_TYPE_2D, .format = vfmt,
                                            .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1}};
                if (vkCreateImageView(c->device, &vi, NULL, &views[0]) != VK_SUCCESS) {
                    printf("probe-blockviews: %s %ux%u: no stand-in view\n", k->name, k->w, k->h);
                    vkEndCommandBuffer(cb);
                    goto out;
                }
            }
            VkImageCopy in = {.srcSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, sl, 0, k->layers},
                              .dstSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, k->layers},
                              .extent = {lw, lh, 1}};   // texels of the compressed source, into the top-left corner
            vkCmdCopyImage(cb, img, VK_IMAGE_LAYOUT_GENERAL, si_img[0], VK_IMAGE_LAYOUT_GENERAL, 1, &in);
            all_barrier(cb);
            VkRenderingAttachmentInfo at = {.sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO, .imageView = views[0],
                                            .imageLayout = VK_IMAGE_LAYOUT_GENERAL, .loadOp = VK_ATTACHMENT_LOAD_OP_LOAD,
                                            .storeOp = VK_ATTACHMENT_STORE_OP_STORE};
            VkRenderingInfo ri = {.sType = VK_STRUCTURE_TYPE_RENDERING_INFO, .renderArea = {{0, 0}, {sw, sh}},
                                  .layerCount = 1, .colorAttachmentCount = 1, .pColorAttachments = &at};
            vkCmdBeginRendering(cb, &ri);
            if (sl == 0) {   // a quarter of layer 0's blocks; the other levels go through unchanged
                VkClearAttachment ca = {.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT, .colorAttachment = 0,
                                        .clearValue.color.uint32 = {SI_CLEAR, SI_CLEAR, SI_CLEAR, SI_CLEAR}};
                VkClearRect cr = {{{0, 0}, {cw, chh}}, 0, 1};
                vkCmdClearAttachments(cb, 1, &ca, 1, &cr);
            }
            vkCmdEndRendering(cb);
            all_barrier(cb);
            VkImageCopy back = {.srcSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, k->layers},
                                .dstSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, sl, 0, k->layers},
                                .extent = {sw, sh, 1}};   // texels of the uncompressed source: the level's blocks
            vkCmdCopyImage(cb, si_img[0], VK_IMAGE_LAYOUT_GENERAL, img, VK_IMAGE_LAYOUT_GENERAL, 1, &back);
            all_barrier(cb);
        } else if (op == OP_COPY) {
            VkBufferImageCopy r = {.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, l, 0, k->layers}, .imageExtent = {lw, lh, 1}};
            vkCmdCopyBufferToImage(cb, src, img, VK_IMAGE_LAYOUT_GENERAL, 1, &r);
        } else if (op == OP_IMAGE) {   // extent in texels of the uncompressed source: the level's blocks
            VkImageCopy r = {.srcSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, k->layers},
                             .dstSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, l, 0, k->layers},
                             .extent = {(lw + 3) / 4, (lh + 3) / 4, 1}};
            vkCmdCopyImage(cb, blk, VK_IMAGE_LAYOUT_GENERAL, img, VK_IMAGE_LAYOUT_GENERAL, 1, &r);
        } else {   // as vk_exec_draw.c makes a block view: the image's COLOR_ATTACHMENT usage is its block views'
            VkImageViewUsageCreateInfo use = {.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO,
                                              .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT};
            VkImageViewCreateInfo vi = {.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO, .pNext = &use, .image = img,
                                        .viewType = VK_IMAGE_VIEW_TYPE_2D, .format = vfmt,
                                        .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, l, 1, 0, 1}};
            if (vkCreateImageView(c->device, &vi, NULL, &views[l]) != VK_SUCCESS) {
                printf("probe-blockviews: %s %ux%u level %u: no block view\n", k->name, k->w, k->h, l);
                vkEndCommandBuffer(cb);
                goto out;
            }
            VkRenderingAttachmentInfo at = {.sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO, .imageView = views[l],
                                            .imageLayout = VK_IMAGE_LAYOUT_GENERAL, .loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR,
                                            .storeOp = VK_ATTACHMENT_STORE_OP_STORE,
                                            .clearValue.color.uint32 = {0x11111111u, 0x22222222u, 0x33333333u, 0x44444444u}};
            VkRenderingInfo ri = {.sType = VK_STRUCTURE_TYPE_RENDERING_INFO, .renderArea = {{0, 0}, {(lw + 3) / 4, (lh + 3) / 4}},
                                  .layerCount = 1, .colorAttachmentCount = 1, .pColorAttachments = &at};
            vkCmdBeginRendering(cb, &ri);
            vkCmdEndRendering(cb);
        }
    }
    all_barrier(cb);
    if (op == OP_COPY || op == OP_IMAGE || op == OP_STANDIN) {   // level 0 as the image holds it
        VkBufferImageCopy r = {.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, k->layers}, .imageExtent = {k->w, k->h, 1}};
        vkCmdCopyImageToBuffer(cb, img, VK_IMAGE_LAYOUT_GENERAL, lvl, 1, &r);
    }
    VkBufferCopy cp = {0, 0, GUARD};
    vkCmdCopyBuffer(cb, guard, back, 1, &cp);
    vkEndCommandBuffer(cb);
    VkFenceCreateInfo fc = {.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    vkCreateFence(c->device, &fc, NULL, &fence);
    VkSubmitInfo sm = {.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO, .commandBufferCount = 1, .pCommandBuffers = &cb};
    VkResult r = vkQueueSubmit(c->gfx_queue, 1, &sm, fence);
    if (r == VK_SUCCESS) r = vkWaitForFences(c->device, 1, &fence, VK_TRUE, 10000000000ull);
    if (r != VK_SUCCESS) {
        printf("probe-blockviews: %s %ux%u %s: submit or wait failed (%d)%s\n", k->name, k->w, k->h, op_name[op], (int)r,
               r == VK_ERROR_DEVICE_LOST ? ": device lost" : "");
        result = r == VK_ERROR_DEVICE_LOST ? -2 : -1;
        goto out;
    }
    vmaInvalidateAllocation(c->vma, back_a, 0, VK_WHOLE_SIZE);
    const uint8_t *p = back_i.pMappedData;
    long changed = 0, first = -1, last = -1;
    for (long i = 0; i < GUARD; i++)
        if (p[i] != PATTERN) {
            if (first < 0) first = i;
            last = i, changed++;
        }
    printf("probe-blockviews: %s %4ux%-4u levels %2u layers %u, %-13s image memory %8llu bytes (alignment %llu): %-48s ", k->name, k->w, k->h,
           k->levels, k->layers, bv ? "block views," : "plain,", (unsigned long long)rq.size, (unsigned long long)rq.alignment, op_name[op]);
    long wrong = 0;
    if (op == OP_COPY || op == OP_IMAGE || op == OP_STANDIN) {
        vmaInvalidateAllocation(c->vma, lvl_a, 0, VK_WHOLE_SIZE);
        const uint8_t *a = src_i.pMappedData, *b = lvl_i.pMappedData;
        for (VkDeviceSize i = 0; i < src_size; i++) {
            uint8_t want = a[i];
            VkDeviceSize blk_i = i / k->bpb, bx = blk_i % bw0, by = blk_i / bw0;   // by counts the rows of every layer
            if (op == OP_STANDIN && by < chh && bx < cw)   // layer 0's cleared quarter: each channel holds SI_CLEAR
                want = k->bpb == 8 || (i % 4) < 2 ? (uint8_t)SI_CLEAR : 0;
            wrong += want != b[i];
        }
    }
    if (wrong) printf("LEVEL 0 READS BACK WRONG: %ld of %llu bytes differ%s", wrong, (unsigned long long)src_size, changed ? "; " : "\n");
    if (!changed && !wrong) printf("ok\n");
    else if (changed) printf("WROTE PAST THE IMAGE: %ld bytes changed, from %lld to %lld bytes behind its end (first bytes %02x %02x %02x %02x)\n",
                changed, (long long)(goff - ioff - rq.size + (VkDeviceSize)first), (long long)(goff - ioff - rq.size + (VkDeviceSize)last + 1),
                p[first], p[first + 1 < GUARD ? first + 1 : first], p[first + 2 < GUARD ? first + 2 : first],
                p[first + 3 < GUARD ? first + 3 : first]);
    result = changed + wrong;
out:
    if (result == -1 && !img) printf("probe-blockviews: %s %ux%u: image creation failed\n", k->name, k->w, k->h);
    vkDeviceWaitIdle(c->device);
    for (int i = 0; i < 16; i++) if (views[i]) vkDestroyImageView(c->device, views[i], NULL);
    for (int i = 0; i < 16; i++) if (si_img[i]) vmaDestroyImage(c->vma, si_img[i], si_a[i]);
    if (fence) vkDestroyFence(c->device, fence, NULL);
    if (cb) vkFreeCommandBuffers(c->device, g_pool, 1, &cb);
    if (src) vmaDestroyBuffer(c->vma, src, src_a);
    if (lvl) vmaDestroyBuffer(c->vma, lvl, lvl_a);
    if (blk) vmaDestroyImage(c->vma, blk, blk_a);
    if (back) vmaDestroyBuffer(c->vma, back, back_a);
    if (guard) vkDestroyBuffer(c->device, guard, NULL);
    if (img) vkDestroyImage(c->device, img, NULL);
    if (mem) vkFreeMemory(c->device, mem, NULL);
    return result;
}

int probe_blockviews(void) {
    if (!vk_init()) { printf("probe-blockviews: FAIL vk_init\n"); return 1; }
    VkCtx *c = vk_ctx();
    printf("probe-blockviews: GPU %s, AMD RDNA 1/2 driver branch %d\n", c->props.deviceName, vk_caps.amd_sc);
    VkCommandPoolCreateInfo pi = {.sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO, .queueFamilyIndex = c->gfx_family,
                                  .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT};
    if (vkCreateCommandPool(c->device, &pi, NULL, &g_pool) != VK_SUCCESS) { printf("probe-blockviews: FAIL command pool\n"); return 1; }
    int over = 0, lost = 0;
    // The passes through block views come last: where they write past the image they may write past the slack too,
    // and what runs after that proves nothing.
    for (int clear = 0; clear < 2 && !lost; clear++)
        for (size_t i = 0; i < sizeof cases / sizeof *cases && !lost; i++)
            for (int bv = clear; bv < 2 && !lost; bv++)
                for (int op = clear ? OP_CLEAR : 0; op < (clear ? OP_N : OP_CLEAR) && !lost; op++) {
                    if (op == OP_STANDIN && bv) continue;   // the stand-in path is for images without block views
                    long n = run_case(c, &cases[i], bv, op);
                    if (n > 0) over++;
                    if (n == -2) lost = 1;
                }
    if (!lost) vkDestroyCommandPool(c->device, g_pool, NULL);
    printf("probe-blockviews: %s\n", lost ? "FAILED: device lost" : over ? "FAILED: the GPU wrote past an image's memory or lost its blocks"
                                                                         : "passed");
    if (!lost) vk_mem_shutdown(), vk_shutdown();
    return lost || over ? 3 : 0;
}
