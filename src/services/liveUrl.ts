/**
 * 直播间 URL 校验与房间号提取（与后端 MissevanClient::extract_room_id 对齐）
 *
 * 规则：https://fm.missevan.com/live/<数字>，仅允许尾部斜杠、查询串或锚点。
 * 提取「/live/ 后第一段」数字作为 room_id —— 路径只允许一段，故第一段即唯一段，
 * 不会出现 /live/123/456 这类前后端取段不一致的静默错配。
 */

export const LIVE_URL_RE =
    /^https:\/\/fm\.missevan\.com\/live\/(\d+)\/?([?#].*)?$/;

/** 从 URL 提取房间号；格式不符返回 null */
export function extractRoomId(url: string): string | null {
    const match = LIVE_URL_RE.exec(url.trim());
    return match ? match[1] : null;
}
