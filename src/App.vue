<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import LoginPage from "./pages/LoginPage.vue";
import MainPage from "./pages/MainPage.vue";

interface UserInfo {
  email: string;
  name: string;
  picture?: string;
}

const user = ref<UserInfo | null>(null);
const checking = ref(true);

onMounted(async () => {
  try {
    user.value = await invoke<UserInfo | null>("check_auth");
  } catch {
    user.value = null;
  } finally {
    checking.value = false;
  }
});
</script>

<template>
  <div v-if="checking" class="loading">Loading...</div>
  <MainPage v-else-if="user" :user="user" @logout="user = null" />
  <LoginPage v-else @login="(u: UserInfo) => (user = u)" />
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: #f5f5f5;
  color: #333;
}
.loading {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100vh;
  font-size: 16px;
  color: #999;
}
</style>
