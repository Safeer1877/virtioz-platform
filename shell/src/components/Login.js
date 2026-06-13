import React, { useState } from 'react';
import { supabase } from '../supabaseClient';
import './workspace.css';
import logo from '../logo.png';

export default function Login() {
  const [isLogin, setIsLogin] = useState(true);
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [username, setUsername] = useState('');
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState('');

  const handleSubmit = async (e) => {
    e.preventDefault();
    setLoading(true);
    setErrorMsg('');

    try {
      if (isLogin) {
        const { error } = await supabase.auth.signInWithPassword({ email, password });
        if (error) throw error;
      } else {
        if (password !== confirmPassword) {
          throw new Error("Passwords do not match");
        }
        
        const { data, error } = await supabase.auth.signUp({ email, password });
        if (error) throw error;
        
        if (data.user) {
          const { error: profileError } = await supabase.from('profiles').insert([
            { id: data.user.id, username, role: 'node' }
          ]);
          if (profileError) throw profileError;
        }
      }
    } catch (error) {
      console.error('Error in auth:', error);
      setErrorMsg(error.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="login-container">
      <div className="login-box">
        <img src={logo} alt="Virtioz Logo" style={{ width: '80px', margin: '0 auto 16px auto', display: 'block', filter: 'drop-shadow(0 4px 12px rgba(244, 114, 182, 0.4))' }} />
        <h1 className="brand-title">Virtioz Shell</h1>
        
        <div className="auth-tabs">
          <div 
            className={`auth-tab ${isLogin ? 'active' : ''}`}
            onClick={() => { setIsLogin(true); setErrorMsg(''); }}
          >
            Log In
          </div>
          <div 
            className={`auth-tab ${!isLogin ? 'active' : ''}`}
            onClick={() => { setIsLogin(false); setErrorMsg(''); }}
          >
            Sign Up
          </div>
        </div>

        {errorMsg && <div style={{ color: '#f43f5e', marginBottom: '20px', textAlign: 'center', fontSize: '13px', fontWeight: '600' }}>{errorMsg}</div>}

        <form onSubmit={handleSubmit} className="login-form">
          {!isLogin && (
            <div className="input-group">
              <label>Username</label>
              <input 
                type="text" 
                value={username} 
                onChange={e => setUsername(e.target.value)} 
                placeholder="Your Name"
                required 
              />
            </div>
          )}

          <div className="input-group">
            <label>Email</label>
            <input 
              type="email" 
              value={email} 
              onChange={e => setEmail(e.target.value)} 
              placeholder="admin@virtioz.io"
              required 
            />
          </div>

          <div className="input-group">
            <label>Password</label>
            <input 
              type="password" 
              value={password} 
              onChange={e => setPassword(e.target.value)} 
              placeholder="••••••••"
              required 
            />
          </div>

          {!isLogin && (
            <div className="input-group">
              <label>Confirm Password</label>
              <input 
                type="password" 
                value={confirmPassword} 
                onChange={e => setConfirmPassword(e.target.value)} 
                placeholder="••••••••"
                required 
              />
            </div>
          )}

          <button type="submit" className="connect-btn" disabled={loading}>
            {loading ? 'Processing...' : (isLogin ? 'Connect to Virtioz' : 'Create Account')}
          </button>
        </form>
      </div>
    </div>
  );
}
