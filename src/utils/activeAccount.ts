import { ref } from "vue";

// 当前 active 账号的 opaque id —— 后端（auth.rs account_key）经 UserInfo.id 下发，
// 前端只当黑盒字符串用，不推算其组成。这是「按账号隔离本地存储」的隔离维度。
//
// 唯一写入点：App.vue 的 watch(user)（user 是全局唯一真相源，切账号 update-user 自动带新 id）。
// scopedKey / 快照 key 前缀 / 迁移守卫都读它。
export const activeAccountId = ref<string>("");

export function getActiveAccountId(): string {
  return activeAccountId.value;
}
