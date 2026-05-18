<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

interface UserInfo {
  email: string;
  name: string;
  picture?: string;
}

const emit = defineEmits<{
  (e: "login", user: UserInfo): void;
}>();

const loading = ref(false);
const error = ref("");

async function handleLogin() {
  loading.value = true;
  error.value = "";
  try {
    const user = await invoke<UserInfo>("start_login");
    emit("login", user);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <div class="login-page">
    <div class="login-card">
      <h1>Tester App</h1>
      <p class="subtitle">Sign in with your @inshot.com account</p>
      <button class="google-btn" @click="handleLogin" :disabled="loading">
        <svg
          v-if="!loading"
          width="18"
          height="18"
          viewBox="0 0 48 48"
          style="margin-right: 8px"
        >
          <path
            fill="#EA4335"
            d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"
          />
          <path
            fill="#4285F4"
            d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"
          />
          <path
            fill="#FBBC05"
            d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"
          />
          <path
            fill="#34A853"
            d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"
          />
        </svg>
        {{ loading ? "Signing in..." : "Sign in with Google" }}
      </button>
      <p v-if="error" class="error">{{ error }}</p>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100vh;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}
.login-card {
  background: white;
  border-radius: 12px;
  padding: 48px;
  text-align: center;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.15);
  min-width: 360px;
}
h1 {
  font-size: 24px;
  margin-bottom: 8px;
  color: #333;
}
.subtitle {
  color: #888;
  margin-bottom: 32px;
  font-size: 14px;
}
.google-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 12px 24px;
  font-size: 14px;
  font-weight: 500;
  border: 1px solid #ddd;
  border-radius: 8px;
  background: white;
  cursor: pointer;
  transition: all 0.2s;
  color: #333;
}
.google-btn:hover:not(:disabled) {
  background: #f7f7f7;
  border-color: #ccc;
}
.google-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.error {
  color: #e53e3e;
  margin-top: 16px;
  font-size: 13px;
}
</style>
